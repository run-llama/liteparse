//! Shared test fixtures for `markdown_layout` submodule tests.

use crate::types::{Anchor, GraphicPrimitive, ParsedPage, ProjectedLine, Rect, TextItem};

pub(crate) fn line(text: &str, x: f32, y: f32, h: f32, size: f32) -> ProjectedLine {
    ProjectedLine {
        text: text.into(),
        bbox: Rect {
            x,
            y,
            width: text.chars().count() as f32 * (size * 0.5),
            height: h,
        },
        anchor: Anchor::Left,
        indent_x: x,
        dominant_font_size: size,
        font_size_is_estimated: false,
        heading_font_size: None,
        dominant_font_name: Some("Arial".into()),
        all_bold: false,
        all_italic: false,
        all_mono: false,
        all_strike: false,
        spans: vec![TextItem::default()],
        region_path: Vec::new(),
        mcid: None,
        in_figure: false,
    }
}

pub(crate) fn page(lines: Vec<ProjectedLine>) -> ParsedPage {
    ParsedPage {
        page_number: 1,
        page_width: 612.0,
        page_height: 792.0,
        text: String::new(),
        markdown: String::new(),
        text_items: vec![],
        projected_lines: lines,
        regions: crate::types::Region::default(),
        graphics: vec![],
        vector_graphics: None,
        figures: vec![],
        struct_nodes: vec![],
        image_refs: vec![],
        complexity: None,
    }
}

pub(crate) fn mono_line(text: &str, y: f32) -> ProjectedLine {
    let mut l = line(text, 50.0, y, 10.0, 10.0);
    l.all_mono = true;
    l
}

/// Build a line whose spans are placed at explicit x positions — used to
/// drive the table detector, which relies on per-span x for cell splitting.
pub(crate) fn line_with_spans(cells: &[(&str, f32)], y: f32, size: f32) -> ProjectedLine {
    let spans: Vec<TextItem> = cells
        .iter()
        .map(|(t, x)| TextItem {
            text: (*t).into(),
            x: *x,
            y,
            width: t.chars().count() as f32 * size * 0.5,
            height: size,
            font_size: Some(size),
            font_name: Some("Arial".into()),
            ..Default::default()
        })
        .collect();
    let min_x = spans.iter().map(|s| s.x).fold(f32::INFINITY, f32::min);
    let max_x = spans
        .iter()
        .map(|s| s.x + s.width)
        .fold(f32::NEG_INFINITY, f32::max);
    ProjectedLine {
        text: cells
            .iter()
            .map(|(t, _)| *t)
            .collect::<Vec<_>>()
            .join("   "),
        bbox: Rect {
            x: min_x,
            y,
            width: (max_x - min_x).max(0.0),
            height: size,
        },
        anchor: Anchor::Left,
        indent_x: min_x,
        dominant_font_size: size,
        font_size_is_estimated: false,
        heading_font_size: None,
        dominant_font_name: Some("Arial".into()),
        all_bold: false,
        all_italic: false,
        all_mono: false,
        all_strike: false,
        spans,
        region_path: Vec::new(),
        mcid: None,
        in_figure: false,
    }
}

/// Build a line whose spans carry explicit per-span font metadata. Lets us
/// exercise the mid-line emphasis pipeline without needing real PDF input.
pub(crate) fn styled_line(spans: &[(&str, f32, Option<&str>)], y: f32, size: f32) -> ProjectedLine {
    let items: Vec<TextItem> = spans
        .iter()
        .map(|(t, x, font)| TextItem {
            text: (*t).into(),
            x: *x,
            y,
            width: t.chars().count() as f32 * size * 0.5,
            height: size,
            font_size: Some(size),
            font_name: font.map(String::from),
            ..Default::default()
        })
        .collect();
    let joined: String = spans
        .iter()
        .map(|(t, _, _)| *t)
        .collect::<Vec<_>>()
        .join(" ");
    let min_x = items.iter().map(|s| s.x).fold(f32::INFINITY, f32::min);
    let max_x = items
        .iter()
        .map(|s| s.x + s.width)
        .fold(f32::NEG_INFINITY, f32::max);
    ProjectedLine {
        text: joined,
        bbox: Rect {
            x: min_x,
            y,
            width: (max_x - min_x).max(0.0),
            height: size,
        },
        anchor: Anchor::Left,
        indent_x: min_x,
        dominant_font_size: size,
        font_size_is_estimated: false,
        heading_font_size: None,
        dominant_font_name: Some("Arial".into()),
        all_bold: false,
        all_italic: false,
        all_mono: false,
        all_strike: false,
        spans: items,
        region_path: Vec::new(),
        mcid: None,
        in_figure: false,
    }
}

pub(crate) fn stroke(x1: f32, y1: f32, x2: f32, y2: f32, width: f32) -> GraphicPrimitive {
    GraphicPrimitive::Stroke {
        x1,
        y1,
        x2,
        y2,
        width,
        color: None,
    }
}

pub(crate) fn page_with_graphics(
    lines: Vec<ProjectedLine>,
    graphics: Vec<GraphicPrimitive>,
) -> ParsedPage {
    let mut p = page(lines);
    p.graphics = graphics;
    p
}

pub(crate) fn header_footer_page(n: usize, header: &str, footer: &str, body: &str) -> ParsedPage {
    // Page height 100 → header band ≤12pt, footer band ≥88pt.
    let lines = vec![
        line(header, 50.0, 5.0, 8.0, 8.0),
        line(body, 50.0, 50.0, 10.0, 10.0),
        line(footer, 50.0, 92.0, 6.0, 6.0),
    ];
    ParsedPage {
        page_number: n,
        page_width: 612.0,
        page_height: 100.0,
        text: String::new(),
        markdown: String::new(),
        text_items: vec![],
        projected_lines: lines,
        regions: crate::types::Region::default(),
        graphics: vec![],
        vector_graphics: None,
        figures: vec![],
        struct_nodes: vec![],
        image_refs: vec![],
        complexity: None,
    }
}

/// Helper: build the four borders of a rectangle as four strokes.
pub(crate) fn rect_borders(x: f32, y: f32, w: f32, h: f32) -> Vec<GraphicPrimitive> {
    vec![
        stroke(x, y, x + w, y, 0.5),         // top
        stroke(x, y + h, x + w, y + h, 0.5), // bottom
        stroke(x, y, x, y + h, 0.5),         // left
        stroke(x + w, y, x + w, y + h, 0.5), // right
    ]
}
