use serde::Serialize;
use std::collections::HashMap;

#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum PdfInput {
    /// Path to a PDF file on disk.
    Path(String),
    /// Raw PDF bytes (e.g. from a network response or in-memory buffer).
    Bytes(Vec<u8>),
}

/// Represents a single text item extracted from a PDF page,
/// including its content, position, size, rotation, and font metadata.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TextItem {
    pub text: String,
    /// Viewport-space coordinates (top-left origin, 72 DPI).
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Rotation in degrees (counter-clockwise, adjusted for page rotation).
    pub rotation: f32,
    pub font_name: Option<String>,
    pub font_size: Option<f32>,
    /// Font size * scale_y from the text matrix — accounts for CTM scaling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_ascent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_descent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_flags: Option<i32>,
    /// Sum of glyph widths (using charcode-based lookup when possible).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_width: Option<f32>,
    /// Whether the font has buggy encoding (private-use codepoints, TT subset, etc.)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub font_is_buggy: bool,
    /// Marked content ID from the PDF structure tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i32>,
    /// Fill color as ARGB hex string (e.g. "ff000000").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    /// Stroke color as ARGB hex string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    /// OCR confidence score (0.0–1.0). None for native PDF text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// Represents an embedded image extracted from a PDF page.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImageItem {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub mime_type: String,
    pub base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

#[doc(hidden)]
#[derive(Debug, Serialize)]
pub struct Page {
    pub page_number: usize,
    pub page_width: f32,
    pub page_height: f32,
    pub text_items: Vec<TextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageItem>,
}

/// Represents a fully parsed page with projected text layout.
#[derive(Debug, Serialize)]
pub struct ParsedPage {
    pub page_number: usize,
    pub page_width: f32,
    pub page_height: f32,
    pub text: String,
    pub text_items: Vec<TextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageItem>,
}

#[doc(hidden)]
#[derive(Debug, Serialize)]
pub enum Snap {
    Left,
    Right,
    Center,
}

#[doc(hidden)]
#[derive(Debug, Serialize)]
pub enum Anchor {
    Left,
    Right,
    Center,
}

#[doc(hidden)]
#[derive(Debug, Serialize)]
pub struct ProjectedTextItem {
    pub item: TextItem,
    pub snap: Snap,
    pub anchor: Anchor,
    pub is_dup: bool,
    pub rendered: bool,
    pub num_spaces: usize,
    pub force_unsnapped: bool,
    pub is_margin_line_number: bool,
    pub rotated: bool,
    pub d: f32,
    pub orig_x: f32,
    pub orig_y: f32,
    pub orig_width: f32,
    pub orig_height: f32,
    pub orig_rotation: f32,
}

#[doc(hidden)]
pub type AnchorMap = HashMap<i32, Vec<(usize, usize)>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> TextItem {
        TextItem {
            text: "hi".into(),
            x: 1.0,
            y: 2.0,
            width: 10.0,
            height: 4.0,
            font_name: Some("Arial".into()),
            font_size: Some(12.0),
            ..Default::default()
        }
    }

    #[test]
    fn text_item_skips_none_fields() {
        let item = sample_item();
        let s = serde_json::to_string(&item).unwrap();
        assert!(!s.contains("font_height"));
        assert!(!s.contains("confidence"));
        assert!(!s.contains("font_is_buggy"));
        assert!(s.contains("\"text\":\"hi\""));
    }

    #[test]
    fn text_item_includes_buggy_flag_when_true() {
        let mut item = sample_item();
        item.font_is_buggy = true;
        let s = serde_json::to_string(&item).unwrap();
        assert!(s.contains("font_is_buggy"));
    }

    #[test]
    fn page_serializes() {
        let p = Page {
            page_number: 1,
            page_width: 100.0,
            page_height: 200.0,
            text_items: vec![sample_item()],
            images: Vec::new(),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"page_number\":1"));
    }

    #[test]
    fn anchor_map_basic() {
        let mut m: AnchorMap = HashMap::new();
        m.entry(5).or_default().push((1, 2));
        assert_eq!(m.get(&5).unwrap()[0], (1, 2));
    }
}
