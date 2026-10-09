use crate::ocr_merge::PageComplexityStats;
use crate::types::{
    DocumentAnnotation, FormField, ParsedPage, Rect, StructureTree, VectorGraphics, XfaPacket,
};
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
    pub trailing_space_generated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonPage {
    pub page: usize,
    /// The document's `/PageLabels` label for this page, omitted when the
    /// PDF defines none. See `ParsedPage::page_label`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_label: Option<String>,
    pub width: f32,
    pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_bounds: Option<Rect>,
    pub text: String,
    pub text_items: Vec<JsonTextItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<PageComplexityStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_graphics: Option<VectorGraphics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<DocumentAnnotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_fields: Option<Vec<FormField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_tree: Option<StructureTree>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<crate::layout::LayoutBlock>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ParseResultJson {
    pub total_pages: u32,
    pub pages: Vec<JsonPage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub page_errors: Vec<crate::types::PageError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ocr_errors: Vec<crate::types::PageError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<JsonImage>,
    #[serde(skip_serializing_if = "is_zero")]
    pub image_error_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_type: Option<i32>,
    /// Raw XFA packets, present only when `extract_xfa_packets` was
    /// requested, so the default CLI JSON stays stable. The document's
    /// `/Info` creator/producer are API-only
    /// (`ParseResult.creator`/`.producer`) for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xfa_packets: Option<Vec<XfaPacket>>,
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
pub(crate) fn build_json(pages: &[ParsedPage], extract_text_metadata: bool) -> ParseResultJson {
    ParseResultJson {
        total_pages: pages.len().min(u32::MAX as usize) as u32,
        images: Vec::new(),
        page_errors: Vec::new(),
        ocr_errors: Vec::new(),
        image_error_count: 0,
        form_type: None,
        xfa_packets: None,
        pages: pages
            .iter()
            .map(|page| JsonPage {
                page: page.page_number,
                page_label: page.page_label.clone(),
                width: page.page_width,
                height: page.page_height,
                content_bounds: page.content_bounds.clone(),
                text: page.text.clone(),
                text_items: page
                    .text_items
                    .iter()
                    .map(|item| {
                        let meta = item.text_metadata(extract_text_metadata);
                        JsonTextItem {
                            text: item.text.clone(),
                            x: item.x,
                            y: item.y,
                            width: item.width,
                            height: item.height,
                            // Not part of TextMetadata: the API surfaces always
                            // expose rotation; only the CLI JSON gates it to
                            // keep the default output byte-stable.
                            rotation: extract_text_metadata.then_some(item.rotation),
                            font_name: item.font_name.clone(),
                            font_size: item.font_size,
                            font_height: meta.font_height,
                            font_ascent: meta.font_ascent,
                            font_descent: meta.font_descent,
                            font_weight: meta.font_weight,
                            text_width: meta.text_width,
                            font_is_buggy: meta.font_is_buggy,
                            mcid: meta.mcid,
                            fill_color: meta.fill_color.map(str::to_owned),
                            stroke_color: meta.stroke_color.map(str::to_owned),
                            char_codes: meta.char_codes.map(<[u32]>::to_vec).unwrap_or_default(),
                            trailing_space_generated: meta
                                .trailing_space_generated
                                .unwrap_or(false),
                            confidence: item.confidence.or(Some(1.0)),
                        }
                    })
                    .collect(),
                complexity: page.complexity.clone(),
                vector_graphics: page.vector_graphics.clone(),
                annotations: page.annotations.clone(),
                form_fields: page.form_fields.clone(),
                structure_tree: page.structure_tree.clone(),
                blocks: page.blocks.clone(),
            })
            .collect(),
    }
}

/// Format complete parse output, including extracted-image metadata and
/// document-level fields (form type, creator/producer, XFA packets). Pixel
/// bytes are written separately by the CLI's `--image-output-dir` option.
pub fn format_json_result(
    result: &crate::parser::ParseResult,
    extract_text_metadata: bool,
) -> Result<String, serde_json::Error> {
    let mut json = build_json(&result.pages, extract_text_metadata);
    json.total_pages = result.total_pages;
    json.images = result
        .images
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
    json.page_errors = result.page_errors.clone();
    json.ocr_errors = result.ocr_errors.clone();
    json.image_error_count = result.image_error_count;
    json.form_type = result.form_type;
    json.xfa_packets = result.xfa_packets.clone();
    serde_json::to_string_pretty(&json)
}

/// Format parsed pages as pretty-printed JSON string.
pub fn format_json(pages: &[ParsedPage]) -> Result<String, serde_json::Error> {
    format_json_with_text_metadata(pages, false)
}

/// Format parsed pages as JSON, optionally including rich PDF text metadata.
pub fn format_json_with_text_metadata(
    pages: &[ParsedPage],
    extract_text_metadata: bool,
) -> Result<String, serde_json::Error> {
    let result = build_json(pages, extract_text_metadata);
    serde_json::to_string_pretty(&result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DocumentAnnotation, ExtractedImage, FormField, ParsedPage, Rect, TextItem};
    use crate::types::{StructureAttributeValue, StructureTree, StructureTreeElement};
    use std::collections::BTreeMap;

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
            page_label: None,
            page_width: 612.0,
            page_height: 792.0,
            content_bounds: None,
            text: "txt".into(),
            markdown: String::new(),
            text_items: items,
            projected_lines: vec![],
            regions: crate::types::Region::default(),
            graphics: vec![],
            vector_graphics: None,
            figures: vec![],
            projected_item_frames: vec![],
            struct_nodes: vec![],
            image_refs: vec![],
            complexity: None,
            annotations: None,
            form_fields: None,
            structure_tree: None,
            blocks: None,
        }
    }

    #[test]
    fn structure_tree_uses_snake_case_and_preserves_typed_attributes() {
        let mut page = page(vec![]);
        page.structure_tree = Some(StructureTree {
            roots: vec![StructureTreeElement {
                element_type: "Figure".into(),
                id: Some("figure-1".into()),
                actual_text: Some("diagram".into()),
                alt_text: Some("A diagram".into()),
                title: None,
                attributes: BTreeMap::from([
                    ("Decorative".into(), StructureAttributeValue::Boolean(false)),
                    ("Width".into(), StructureAttributeValue::Number(42.5)),
                    (
                        "Placement".into(),
                        StructureAttributeValue::String("Block".into()),
                    ),
                ]),
                marked_content_ids: vec![7],
                children: vec![],
                annotations: vec![],
            }],
        });

        let value = serde_json::to_value(build_json(&[page], false)).unwrap();
        let root = &value["pages"][0]["structure_tree"]["roots"][0];
        assert_eq!(root["type"], "Figure");
        assert_eq!(root["actual_text"], "diagram");
        assert_eq!(root["marked_content_ids"][0], 7);
        assert_eq!(root["attributes"]["Decorative"], false);
        assert_eq!(root["attributes"]["Width"], 42.5);
        assert_eq!(root["attributes"]["Placement"], "Block");
        assert!(root.get("actualText").is_none());
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
        text_item.trailing_space_generated = true;

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
        assert_eq!(item["trailing_space_generated"], true);
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
        text_item.trailing_space_generated = true;

        let value: serde_json::Value =
            serde_json::from_str(&format_json(&[page(vec![text_item])]).unwrap()).unwrap();
        let item = &value["pages"][0]["text_items"][0];
        assert!(item.get("rotation").is_none());
        assert!(item.get("font_height").is_none());
        assert!(item.get("font_is_buggy").is_none());
        assert!(item.get("mcid").is_none());
        assert!(item.get("char_codes").is_none());
        assert!(item.get("trailing_space_generated").is_none());
    }

    #[test]
    fn test_format_json_result_includes_image_metadata_and_errors() {
        let image = ExtractedImage {
            id: "p2_0".into(),
            name: "img_p2_1.jpg".into(),
            path: Some("/tmp/images/img_p2_1.jpg".into()),
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
            bytes: std::sync::Arc::new(vec![1, 2, 3]),
        };
        let result = crate::parser::ParseResult {
            total_pages: 3,
            pages: vec![],
            page_errors: vec![crate::types::PageError {
                page_number: 3,
                message: "page extraction failed".into(),
            }],
            ocr_errors: vec![crate::types::PageError {
                page_number: 2,
                message: "OCR worker failed".into(),
            }],
            text: String::new(),
            outline: vec![],
            images: vec![image],
            screenshots: vec![],
            image_error_count: 2,
            form_type: None,
            creator: Some("LibreOffice".into()),
            producer: Some("LibreOffice 7.4".into()),
            doc_meta: Some(crate::types::DocumentMetadata::default()),
            xfa_packets: Some(vec![crate::types::XfaPacket {
                index: 0,
                name: Some("datasets".into()),
                content_length: 11,
                content: Some("<xml></xml>".into()),
            }]),
        };
        let value: serde_json::Value =
            serde_json::from_str(&format_json_result(&result, false).unwrap()).unwrap();
        assert_eq!(value["total_pages"], 3);
        assert!(value.get("creator").is_none());
        assert!(value.get("producer").is_none());
        assert!(value.get("doc_meta").is_none());
        assert_eq!(value["xfa_packets"][0]["name"], "datasets");
        assert_eq!(value["xfa_packets"][0]["content_length"], 11);
        assert_eq!(value["images"][0]["bbox"]["x"], 10.0);
        assert_eq!(value["images"][0]["width"], 640);
        assert_eq!(value["images"][0]["rotation"], 90.0);
        assert_eq!(value["images"][0]["duplicate_of"], "p1_0");
        assert!(value["images"][0].get("bytes").is_none());
        assert_eq!(value["image_error_count"], 2);
        assert_eq!(value["page_errors"][0]["page"], 3);
        assert_eq!(value["page_errors"][0]["message"], "page extraction failed");
        assert_eq!(value["ocr_errors"][0]["page"], 2);
        assert_eq!(value["ocr_errors"][0]["message"], "OCR worker failed");
    }

    #[test]
    fn vector_graphics_is_absent_by_default_and_serialized_when_present() {
        let default_json = format_json(&[page(vec![])]).unwrap();
        assert!(!default_json.contains("vector_graphics"));

        let mut with_vectors = page(vec![]);
        with_vectors.vector_graphics = Some(VectorGraphics {
            shapes: vec![crate::types::VectorShape {
                bbox: crate::types::Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
                stroke: true,
                stroke_color: Some("ff000000".into()),
                fill: false,
                fill_color: None,
                has_curve: true,
            }],
            lines: vec![],
        });
        let enabled_json = format_json(&[with_vectors]).unwrap();
        assert!(enabled_json.contains("\"vector_graphics\""));
        assert!(enabled_json.contains("\"has_curve\": true"));
        assert!(enabled_json.contains("\"stroke_color\": \"ff000000\""));
    }

    #[test]
    fn test_annotations_are_omitted_when_disabled() {
        let value = serde_json::to_value(build_json(&[page(vec![])], false)).unwrap();
        assert!(value["pages"][0].get("annotations").is_none());
    }

    #[test]
    fn test_annotations_are_serialized_when_enabled() {
        let mut parsed_page = page(vec![]);
        parsed_page.annotations = Some(vec![DocumentAnnotation {
            subtype: "highlight".into(),
            contents: Some("review this".into()),
            created: None,
            modified: None,
            title: Some("Reviewer".into()),
            rect: Some(Rect {
                x: 10.0,
                y: 20.0,
                width: 90.0,
                height: 20.0,
            }),
            quadpoint_rects: vec![],
            uri: None,
        }]);
        let value = serde_json::to_value(build_json(&[parsed_page], false)).unwrap();
        assert_eq!(value["pages"][0]["annotations"][0]["subtype"], "highlight");
        assert_eq!(value["pages"][0]["annotations"][0]["rect"]["width"], 90.0);
    }

    #[test]
    fn form_fields_use_the_public_snake_case_schema() {
        let mut parsed_page = page(vec![]);
        parsed_page.form_fields = Some(vec![FormField {
            id: "full_name".into(),
            field_type: "text".into(),
            page: 1,
            annotation_index: 2,
            widget_index: 0,
            object_number: Some(42),
            name: Some("full_name".into()),
            alternate_name: Some("Full name".into()),
            value: Some("Ada".into()),
            export_value: None,
            field_flags: 0,
            control_count: None,
            control_index: None,
            checked: None,
            rect: None,
            options: vec![],
            selected_options: vec![],
        }]);
        let value = serde_json::to_value(build_json(&[parsed_page], false)).unwrap();
        let field = &value["pages"][0]["form_fields"][0];
        assert_eq!(field["type"], "text");
        assert_eq!(field["annotation_index"], 2);
        assert_eq!(field["alternate_name"], "Full name");
        assert!(field.get("field_type").is_none());
    }
}
