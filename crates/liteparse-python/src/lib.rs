use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use liteparse::config::{CropBox, ImageMode, LiteParseConfig, OutputFormat};
use liteparse::types::PdfInput;

mod cli;

// ---------------------------------------------------------------------------
// Python type wrappers
// ---------------------------------------------------------------------------

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyWordBox {
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
}

#[pymethods]
impl PyWordBox {
    fn __repr__(&self) -> String {
        format!(
            "WordBox(text={:?}, x={}, y={}, width={}, height={})",
            self.text, self.x, self.y, self.width, self.height
        )
    }
}

impl PyWordBox {
    fn from_rust(word: liteparse::types::WordBox) -> Self {
        Self {
            text: word.text,
            x: word.x as f64,
            y: word.y as f64,
            width: word.width as f64,
            height: word.height as f64,
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyTextItem {
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
    #[pyo3(get)]
    font_name: Option<String>,
    #[pyo3(get)]
    font_size: Option<f64>,
    #[pyo3(get)]
    font_height: Option<f64>,
    #[pyo3(get)]
    font_ascent: Option<f64>,
    #[pyo3(get)]
    font_descent: Option<f64>,
    #[pyo3(get)]
    font_weight: Option<i32>,
    #[pyo3(get)]
    text_width: Option<f64>,
    #[pyo3(get)]
    font_is_buggy: bool,
    #[pyo3(get)]
    mcid: Option<i32>,
    /// Fill color as an eight-character ARGB hex string.
    #[pyo3(get)]
    fill_color: Option<String>,
    /// Stroke color as an eight-character ARGB hex string.
    #[pyo3(get)]
    stroke_color: Option<String>,
    /// Raw PDF content-stream character codes for the source glyphs.
    #[pyo3(get)]
    char_codes: Vec<u32>,
    /// True when the trailing source space was synthesized by PDFium.
    #[pyo3(get)]
    trailing_space_generated: bool,
    /// OCR confidence score (0.0-1.0). None for native PDF text.
    #[pyo3(get)]
    confidence: Option<f64>,
    /// Rotation in degrees (viewport space). Defaults to 0.
    #[pyo3(get)]
    rotation: f64,
    /// Per-word sub-boxes for attribution. Empty unless the parse was
    /// configured with `emit_word_boxes=True`.
    #[pyo3(get)]
    words: Vec<PyWordBox>,
}

#[pymethods]
impl PyTextItem {
    fn __repr__(&self) -> String {
        format!(
            "TextItem(text={:?}, x={}, y={}, width={}, height={})",
            self.text, self.x, self.y, self.width, self.height
        )
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyAnnotationRect {
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
}

impl PyAnnotationRect {
    fn from_rust(rect: liteparse::types::Rect) -> Self {
        Self {
            x: rect.x as f64,
            y: rect.y as f64,
            width: rect.width as f64,
            height: rect.height as f64,
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyDocumentAnnotation {
    #[pyo3(get)]
    subtype: String,
    #[pyo3(get)]
    contents: Option<String>,
    #[pyo3(get)]
    created: Option<String>,
    #[pyo3(get)]
    modified: Option<String>,
    #[pyo3(get)]
    title: Option<String>,
    #[pyo3(get)]
    rect: Option<PyAnnotationRect>,
    #[pyo3(get)]
    quadpoint_rects: Vec<PyAnnotationRect>,
    #[pyo3(get)]
    uri: Option<String>,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyStructureAttribute {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    boolean_value: Option<bool>,
    #[pyo3(get)]
    number_value: Option<f64>,
    #[pyo3(get)]
    string_value: Option<String>,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyStructureTreeElement {
    #[pyo3(get)]
    element_type: String,
    #[pyo3(get)]
    id: Option<String>,
    #[pyo3(get)]
    actual_text: Option<String>,
    #[pyo3(get)]
    alt_text: Option<String>,
    #[pyo3(get)]
    title: Option<String>,
    #[pyo3(get)]
    attributes: Vec<PyStructureAttribute>,
    #[pyo3(get)]
    marked_content_ids: Vec<i32>,
    #[pyo3(get)]
    children: Vec<PyStructureTreeElement>,
    #[pyo3(get)]
    annotations: Vec<PyDocumentAnnotation>,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyStructureTree {
    #[pyo3(get)]
    roots: Vec<PyStructureTreeElement>,
}

impl PyStructureTreeElement {
    fn from_rust(element: liteparse::types::StructureTreeElement) -> Self {
        Self {
            element_type: element.element_type,
            id: element.id,
            actual_text: element.actual_text,
            alt_text: element.alt_text,
            title: element.title,
            attributes: element
                .attributes
                .into_iter()
                .map(|(name, value)| {
                    let (boolean_value, number_value, string_value) = match value {
                        liteparse::types::StructureAttributeValue::Boolean(value) => {
                            (Some(value), None, None)
                        }
                        liteparse::types::StructureAttributeValue::Number(value) => {
                            (None, Some(f64::from(value)), None)
                        }
                        liteparse::types::StructureAttributeValue::String(value) => {
                            (None, None, Some(value))
                        }
                    };
                    PyStructureAttribute {
                        name,
                        boolean_value,
                        number_value,
                        string_value,
                    }
                })
                .collect(),
            marked_content_ids: element.marked_content_ids,
            children: element.children.into_iter().map(Self::from_rust).collect(),
            annotations: element
                .annotations
                .into_iter()
                .map(PyDocumentAnnotation::from_rust)
                .collect(),
        }
    }
}

impl PyDocumentAnnotation {
    fn from_rust(annotation: liteparse::types::DocumentAnnotation) -> Self {
        Self {
            subtype: annotation.subtype,
            contents: annotation.contents,
            created: annotation.created,
            modified: annotation.modified,
            title: annotation.title,
            rect: annotation.rect.map(PyAnnotationRect::from_rust),
            quadpoint_rects: annotation
                .quadpoint_rects
                .into_iter()
                .map(PyAnnotationRect::from_rust)
                .collect(),
            uri: annotation.uri,
        }
    }
}

/// One table cell: its rendered text and the region it occupied.
///
/// `bbox` is `None` for cells with no ink behind them — padding inserted to
/// square off a ragged grid, or halves of a merged run split at an estimated
/// position rather than an observed boundary.
#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyLayoutCell {
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    bbox: Option<PyAnnotationRect>,
    /// Indices into the page's returned `text_items`, in reading order,
    /// never repeating within one cell; empty for padding cells.
    #[pyo3(get)]
    text_item_indices: Vec<usize>,
}

impl PyLayoutCell {
    fn from_rust(cell: liteparse::layout::LayoutCell) -> Self {
        Self {
            text: cell.text,
            bbox: cell.bbox.map(PyAnnotationRect::from_rust),
            text_item_indices: cell.text_item_indices,
        }
    }
}

/// A classified block plus where it sits on the page.
///
/// `kind` discriminates the block; every field that doesn't apply to a block's
/// kind is `None` (or `False` for the flags). Blocks appear in reading order,
/// matching the order the markdown renderer emits them.
#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyLayoutBlock {
    /// One of `heading`, `paragraph`, `list_item`, `code`, `table`,
    /// `grid_fallback`, `rule`, `figure`.
    #[pyo3(get)]
    kind: String,
    /// Indices into the page's returned `text_items`, sorted and deduped;
    /// empty for text-less blocks. For a `table` block, the union of its
    /// cells' indices.
    #[pyo3(get)]
    text_item_indices: Vec<usize>,
    /// Rendered text for the text-bearing kinds (`heading`, `paragraph`,
    /// `list_item`). Table text lives in `header`/`rows`; code and grid text in
    /// `lines`.
    #[pyo3(get)]
    text: Option<String>,
    /// Heading level (1–6), or list nesting depth for `list_item`.
    #[pyo3(get)]
    level: Option<u8>,
    /// Whether the block's text is uniformly bold / italic. `paragraph` and
    /// `list_item` only.
    #[pyo3(get)]
    bold: bool,
    #[pyo3(get)]
    italic: bool,
    /// `list_item`: whether the list is ordered, and the original marker as it
    /// appeared on the page (`138.`, `iii)`, `•`).
    #[pyo3(get)]
    ordered: Option<bool>,
    #[pyo3(get)]
    marker: Option<String>,
    /// Verbatim source lines for `code` and `grid_fallback`.
    #[pyo3(get)]
    lines: Option<Vec<String>>,
    /// Best-effort language hint for `code`.
    #[pyo3(get)]
    lang: Option<String>,
    /// `table`: the header row, when one was detected.
    #[pyo3(get)]
    header: Option<Vec<PyLayoutCell>>,
    /// `table`: the body rows.
    #[pyo3(get)]
    rows: Option<Vec<Vec<PyLayoutCell>>>,
    /// `figure`: the image's page-scoped id and encoded format, matching the
    /// `img_{id}.{format}` target the markdown renderer emits.
    #[pyo3(get)]
    id: Option<String>,
    #[pyo3(get)]
    format: Option<String>,
    /// Region of the page this block occupies, in the same top-left, 72-DPI
    /// viewport space as `text_items`. `None` when the block has no page
    /// geometry behind it.
    #[pyo3(get)]
    bbox: Option<PyAnnotationRect>,
}

impl PyLayoutBlock {
    fn from_rust(block: liteparse::layout::LayoutBlock) -> Self {
        Self {
            kind: block.kind.to_string(),
            text_item_indices: block.text_item_indices,
            text: block.text,
            level: block.level,
            bold: block.bold,
            italic: block.italic,
            ordered: block.ordered,
            marker: block.marker,
            lines: block.lines,
            lang: block.lang,
            header: block
                .header
                .map(|cells| cells.into_iter().map(PyLayoutCell::from_rust).collect()),
            rows: block.rows.map(|rows| {
                rows.into_iter()
                    .map(|row| row.into_iter().map(PyLayoutCell::from_rust).collect())
                    .collect()
            }),
            id: block.id,
            format: block.format,
            bbox: block.bbox.map(PyAnnotationRect::from_rust),
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyFormField {
    #[pyo3(get)]
    id: String,
    #[pyo3(get)]
    field_type: String,
    #[pyo3(get)]
    page: u32,
    #[pyo3(get)]
    annotation_index: i32,
    #[pyo3(get)]
    widget_index: i32,
    #[pyo3(get)]
    object_number: Option<i32>,
    #[pyo3(get)]
    name: Option<String>,
    #[pyo3(get)]
    alternate_name: Option<String>,
    #[pyo3(get)]
    value: Option<String>,
    #[pyo3(get)]
    export_value: Option<String>,
    #[pyo3(get)]
    field_flags: i32,
    #[pyo3(get)]
    control_count: Option<i32>,
    #[pyo3(get)]
    control_index: Option<i32>,
    #[pyo3(get)]
    checked: Option<bool>,
    #[pyo3(get)]
    rect: Option<PyAnnotationRect>,
    #[pyo3(get)]
    options: Vec<String>,
    #[pyo3(get)]
    selected_options: Vec<String>,
}

impl PyFormField {
    fn from_rust(field: liteparse::types::FormField) -> Self {
        Self {
            id: field.id,
            field_type: field.field_type,
            page: field.page,
            annotation_index: field.annotation_index,
            widget_index: field.widget_index,
            object_number: field.object_number,
            name: field.name,
            alternate_name: field.alternate_name,
            value: field.value,
            export_value: field.export_value,
            field_flags: field.field_flags,
            control_count: field.control_count,
            control_index: field.control_index,
            checked: field.checked,
            rect: field.rect.map(PyAnnotationRect::from_rust),
            options: field.options,
            selected_options: field.selected_options,
        }
    }
}

impl PyTextItem {
    fn to_rust(&self) -> liteparse::types::TextItem {
        liteparse::types::TextItem {
            text: self.text.clone(),
            x: self.x as f32,
            y: self.y as f32,
            width: self.width as f32,
            height: self.height as f32,
            rotation: self.rotation as f32,
            font_name: self.font_name.clone(),
            font_size: self.font_size.map(|v| v as f32),
            font_height: self.font_height.map(|v| v as f32),
            font_ascent: self.font_ascent.map(|v| v as f32),
            font_descent: self.font_descent.map(|v| v as f32),
            font_weight: self.font_weight,
            text_width: self.text_width.map(|v| v as f32),
            font_is_buggy: self.font_is_buggy,
            mcid: self.mcid,
            fill_color: self.fill_color.clone(),
            stroke_color: self.stroke_color.clone(),
            char_codes: self.char_codes.clone(),
            trailing_space_generated: self.trailing_space_generated,
            confidence: self.confidence.map(|v| v as f32),
            ..Default::default()
        }
    }

    fn from_rust(item: liteparse::types::TextItem) -> Self {
        Self {
            text: item.text,
            x: item.x as f64,
            y: item.y as f64,
            width: item.width as f64,
            height: item.height as f64,
            font_name: item.font_name,
            font_size: item.font_size.map(|v| v as f64),
            font_height: item.font_height.map(|v| v as f64),
            font_ascent: item.font_ascent.map(|v| v as f64),
            font_descent: item.font_descent.map(|v| v as f64),
            font_weight: item.font_weight,
            text_width: item.text_width.map(|v| v as f64),
            font_is_buggy: item.font_is_buggy,
            mcid: item.mcid,
            fill_color: item.fill_color,
            stroke_color: item.stroke_color,
            char_codes: item.char_codes,
            trailing_space_generated: item.trailing_space_generated,
            confidence: item.confidence.map(|v| v as f64),
            rotation: item.rotation as f64,
            words: item.words.into_iter().map(PyWordBox::from_rust).collect(),
        }
    }

    /// `from_rust` with the rich-metadata fields taken from the core-gated
    /// [`liteparse::types::TextMetadata`] view, so the "what counts as text
    /// metadata" list lives in one place instead of per binding.
    fn from_rust_for_output(item: liteparse::types::TextItem, extract_text_metadata: bool) -> Self {
        let meta = item.text_metadata(extract_text_metadata);
        let (fill_color, stroke_color, char_codes) = (
            meta.fill_color.map(str::to_owned),
            meta.stroke_color.map(str::to_owned),
            meta.char_codes.map(<[u32]>::to_vec),
        );
        Self {
            font_height: meta.font_height.map(|v| v as f64),
            font_ascent: meta.font_ascent.map(|v| v as f64),
            font_descent: meta.font_descent.map(|v| v as f64),
            font_weight: meta.font_weight,
            text_width: meta.text_width.map(|v| v as f64),
            font_is_buggy: meta.font_is_buggy.unwrap_or(false),
            mcid: meta.mcid,
            fill_color,
            stroke_color,
            char_codes: char_codes.unwrap_or_default(),
            trailing_space_generated: meta.trailing_space_generated.unwrap_or(false),
            ..Self::from_rust(item)
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyParsedPage {
    #[pyo3(get)]
    page_num: u32,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
    #[pyo3(get)]
    content_bounds: Option<PyRect>,
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    markdown: String,
    #[pyo3(get)]
    text_items: Vec<PyTextItem>,
    #[pyo3(get)]
    complexity: Option<PyPageComplexityStats>,
    #[pyo3(get)]
    vector_graphics: Option<PyVectorGraphics>,
    #[pyo3(get)]
    annotations: Option<Vec<PyDocumentAnnotation>>,
    #[pyo3(get)]
    form_fields: Option<Vec<PyFormField>>,
    #[pyo3(get)]
    structure_tree: Option<PyStructureTree>,
    #[pyo3(get)]
    blocks: Option<Vec<PyLayoutBlock>>,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyRect {
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyVectorShape {
    #[pyo3(get)]
    bbox: PyRect,
    #[pyo3(get)]
    stroke: bool,
    #[pyo3(get)]
    stroke_color: Option<String>,
    #[pyo3(get)]
    fill: bool,
    #[pyo3(get)]
    fill_color: Option<String>,
    #[pyo3(get)]
    has_curve: bool,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyVectorLine {
    #[pyo3(get)]
    x1: f64,
    #[pyo3(get)]
    y1: f64,
    #[pyo3(get)]
    x2: f64,
    #[pyo3(get)]
    y2: f64,
    #[pyo3(get)]
    stroke: bool,
    #[pyo3(get)]
    stroke_width: Option<f64>,
    #[pyo3(get)]
    stroke_color: Option<String>,
    #[pyo3(get)]
    fill: bool,
    #[pyo3(get)]
    fill_color: Option<String>,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyVectorGraphics {
    #[pyo3(get)]
    shapes: Vec<PyVectorShape>,
    #[pyo3(get)]
    lines: Vec<PyVectorLine>,
}

impl PyVectorGraphics {
    fn from_rust(v: liteparse::types::VectorGraphics) -> Self {
        Self {
            shapes: v
                .shapes
                .into_iter()
                .map(|s| PyVectorShape {
                    bbox: PyRect {
                        x: s.bbox.x as f64,
                        y: s.bbox.y as f64,
                        width: s.bbox.width as f64,
                        height: s.bbox.height as f64,
                    },
                    stroke: s.stroke,
                    stroke_color: s.stroke_color,
                    fill: s.fill,
                    fill_color: s.fill_color,
                    has_curve: s.has_curve,
                })
                .collect(),
            lines: v
                .lines
                .into_iter()
                .map(|l| PyVectorLine {
                    x1: l.x1 as f64,
                    y1: l.y1 as f64,
                    x2: l.x2 as f64,
                    y2: l.y2 as f64,
                    stroke: l.stroke,
                    stroke_width: l.stroke_width.map(f64::from),
                    stroke_color: l.stroke_color,
                    fill: l.fill,
                    fill_color: l.fill_color,
                })
                .collect(),
        }
    }
}

#[pymethods]
impl PyParsedPage {
    fn __repr__(&self) -> String {
        format!(
            "ParsedPage(page_num={}, width={}, height={}, text_items={})",
            self.page_num,
            self.width,
            self.height,
            self.text_items.len()
        )
    }
}

impl PyParsedPage {
    fn from_rust(page: liteparse::types::ParsedPage, extract_text_metadata: bool) -> Self {
        Self {
            page_num: page.page_number as u32,
            width: page.page_width as f64,
            height: page.page_height as f64,
            content_bounds: page.content_bounds.as_ref().map(|b| PyRect {
                x: b.x as f64,
                y: b.y as f64,
                width: b.width as f64,
                height: b.height as f64,
            }),
            text: page.text,
            markdown: page.markdown,
            text_items: page
                .text_items
                .into_iter()
                .map(|item| PyTextItem::from_rust_for_output(item, extract_text_metadata))
                .collect(),
            complexity: page
                .complexity
                .as_ref()
                .map(PyPageComplexityStats::from_rust),
            vector_graphics: page.vector_graphics.map(PyVectorGraphics::from_rust),
            annotations: page.annotations.map(|annotations| {
                annotations
                    .into_iter()
                    .map(PyDocumentAnnotation::from_rust)
                    .collect()
            }),
            form_fields: page
                .form_fields
                .map(|fields| fields.into_iter().map(PyFormField::from_rust).collect()),
            structure_tree: page.structure_tree.map(|tree| PyStructureTree {
                roots: tree
                    .roots
                    .into_iter()
                    .map(PyStructureTreeElement::from_rust)
                    .collect(),
            }),
            blocks: page
                .blocks
                .map(|blocks| blocks.into_iter().map(PyLayoutBlock::from_rust).collect()),
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyParseResult {
    #[pyo3(get)]
    total_pages: u32,
    #[pyo3(get)]
    pages: Vec<PyParsedPage>,
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    images: Vec<PyExtractedImage>,
    #[pyo3(get)]
    screenshots: Vec<PyScreenshotResult>,
    #[pyo3(get)]
    image_error_count: u32,
    #[pyo3(get)]
    page_errors: Vec<PyPageError>,
    #[pyo3(get)]
    form_type: Option<i32>,
    #[pyo3(get)]
    creator: Option<String>,
    #[pyo3(get)]
    producer: Option<String>,
    #[pyo3(get)]
    doc_meta: Option<PyDocumentMetadata>,
    #[pyo3(get)]
    xfa_packets: Option<Vec<PyXfaPacket>>,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyPageError {
    #[pyo3(get)]
    page_num: u32,
    #[pyo3(get)]
    message: String,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyDocumentMetadata {
    #[pyo3(get)]
    creation_date: Option<String>,
    #[pyo3(get)]
    mod_date: Option<String>,
    #[pyo3(get)]
    file_version: Option<i32>,
    #[pyo3(get)]
    is_encrypted: Option<bool>,
    #[pyo3(get)]
    security_handler_revision: Option<i32>,
    #[pyo3(get)]
    permissions: Option<u64>,
    #[pyo3(get)]
    eof_section_count: Option<u32>,
    #[pyo3(get)]
    startxref_count: Option<u32>,
    #[pyo3(get)]
    trailer_id_pair_differs: Option<bool>,
    #[pyo3(get)]
    raw_file_size: Option<u64>,
    #[pyo3(get)]
    xmp: Option<String>,
    #[pyo3(get)]
    xmp_truncated: Option<bool>,
    #[pyo3(get)]
    signature_count: Option<u32>,
    #[pyo3(get)]
    signature_byte_range_reaches_eof: Option<bool>,
}

impl From<liteparse::types::DocumentMetadata> for PyDocumentMetadata {
    fn from(metadata: liteparse::types::DocumentMetadata) -> Self {
        Self {
            creation_date: metadata.creation_date,
            mod_date: metadata.mod_date,
            file_version: metadata.file_version,
            is_encrypted: metadata.is_encrypted,
            security_handler_revision: metadata.security_handler_revision,
            permissions: metadata.permissions,
            eof_section_count: metadata.eof_section_count,
            startxref_count: metadata.startxref_count,
            trailer_id_pair_differs: metadata.trailer_id_pair_differs,
            raw_file_size: metadata.raw_file_size,
            xmp: metadata.xmp,
            xmp_truncated: metadata.xmp_truncated,
            signature_count: metadata.signature_count,
            signature_byte_range_reaches_eof: metadata.signature_byte_range_reaches_eof,
        }
    }
}

/// One raw packet from an XFA form document's `/XFA` array.
#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyXfaPacket {
    #[pyo3(get)]
    index: u32,
    #[pyo3(get)]
    name: Option<String>,
    #[pyo3(get)]
    content_length: u32,
    #[pyo3(get)]
    content: Option<String>,
}

#[pymethods]
impl PyXfaPacket {
    fn __repr__(&self) -> String {
        format!(
            "XfaPacket(index={}, name={:?}, content_length={})",
            self.index, self.name, self.content_length
        )
    }
}

#[pymethods]
impl PyParseResult {
    #[getter]
    fn num_pages(&self) -> usize {
        self.pages.len()
    }

    fn get_page(&self, page_num: u32) -> Option<PyParsedPage> {
        self.pages.iter().find(|p| p.page_num == page_num).cloned()
    }

    fn __repr__(&self) -> String {
        format!(
            "ParseResult(pages={}, text_len={}, images={})",
            self.pages.len(),
            self.text.len(),
            self.images.len()
        )
    }
}

impl PyParseResult {
    fn from_rust(result: liteparse::parser::ParseResult, extract_text_metadata: bool) -> Self {
        Self {
            total_pages: result.total_pages,
            pages: result
                .pages
                .into_iter()
                .map(|page| PyParsedPage::from_rust(page, extract_text_metadata))
                .collect(),
            text: result.text,
            images: result
                .images
                .into_iter()
                .map(PyExtractedImage::from_rust)
                .collect(),
            screenshots: result
                .screenshots
                .into_iter()
                .map(PyScreenshotResult::from_rust)
                .collect(),
            image_error_count: result.image_error_count,
            page_errors: result
                .page_errors
                .into_iter()
                .map(|error| PyPageError {
                    page_num: error.page_number,
                    message: error.message,
                })
                .collect(),
            form_type: result.form_type,
            creator: result.creator,
            producer: result.producer,
            doc_meta: result.doc_meta.map(Into::into),
            xfa_packets: result.xfa_packets.map(|packets| {
                packets
                    .into_iter()
                    .map(|packet| PyXfaPacket {
                        index: packet.index,
                        name: packet.name,
                        content_length: packet.content_length,
                        content: packet.content,
                    })
                    .collect()
            }),
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyExtractedImage {
    #[pyo3(get)]
    id: String,
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    path: Option<String>,
    #[pyo3(get)]
    page: u32,
    #[pyo3(get)]
    bbox: PyImageRect,
    #[pyo3(get)]
    width: u32,
    #[pyo3(get)]
    height: u32,
    #[pyo3(get)]
    rotation: f32,
    #[pyo3(get)]
    format: String,
    #[pyo3(get)]
    duplicate_of: Option<String>,
    bytes_buffer: std::sync::Arc<Vec<u8>>,
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyImageRect {
    #[pyo3(get)]
    x: f32,
    #[pyo3(get)]
    y: f32,
    #[pyo3(get)]
    width: f32,
    #[pyo3(get)]
    height: f32,
}

#[pymethods]
impl PyExtractedImage {
    #[getter]
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.bytes_buffer.as_slice())
    }

    fn __repr__(&self) -> String {
        format!(
            "ExtractedImage(id='{}', page={}, format='{}', bytes_len={})",
            self.id,
            self.page,
            self.format,
            self.bytes_buffer.len()
        )
    }
}

impl PyExtractedImage {
    fn from_rust(img: liteparse::types::ExtractedImage) -> Self {
        Self {
            id: img.id,
            name: img.name,
            path: img.path,
            page: img.page,
            bbox: PyImageRect {
                x: img.bbox.x,
                y: img.bbox.y,
                width: img.bbox.width,
                height: img.bbox.height,
            },
            width: img.width,
            height: img.height,
            rotation: img.rotation,
            format: img.format,
            duplicate_of: img.duplicate_of,
            bytes_buffer: img.bytes,
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyScreenshotResult {
    #[pyo3(get)]
    page_num: u32,
    #[pyo3(get)]
    width: u32,
    #[pyo3(get)]
    height: u32,
    image_buffer: Vec<u8>,
    #[pyo3(get)]
    is_solid_fill: bool,
    #[pyo3(get)]
    rects: Vec<PyScreenshotRect>,
}

/// One solid rectangle (or line) detected in a rendered page bitmap, in
/// viewport coords (top-left origin, 72 DPI).
#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyScreenshotRect {
    #[pyo3(get)]
    x: f64,
    #[pyo3(get)]
    y: f64,
    #[pyo3(get)]
    width: f64,
    #[pyo3(get)]
    height: f64,
    #[pyo3(get)]
    color: String,
    #[pyo3(get)]
    is_line: bool,
}

#[pymethods]
impl PyScreenshotRect {
    fn __repr__(&self) -> String {
        format!(
            "ScreenshotRect(x={}, y={}, width={}, height={}, color={}, is_line={})",
            self.x, self.y, self.width, self.height, self.color, self.is_line
        )
    }
}

impl PyScreenshotResult {
    fn from_rust(result: liteparse::parser::ScreenshotResult) -> Self {
        Self {
            page_num: result.page_num,
            width: result.width,
            height: result.height,
            image_buffer: result.image_bytes,
            is_solid_fill: result.is_solid_fill,
            rects: result
                .rects
                .into_iter()
                .map(|rect| PyScreenshotRect {
                    x: rect.x as f64,
                    y: rect.y as f64,
                    width: rect.width as f64,
                    height: rect.height as f64,
                    color: rect.color,
                    is_line: rect.is_line,
                })
                .collect(),
        }
    }
}

#[pymethods]
impl PyScreenshotResult {
    #[getter]
    fn image_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.image_buffer)
    }

    fn __repr__(&self) -> String {
        format!(
            "ScreenshotResult(page_num={}, width={}, height={})",
            self.page_num, self.width, self.height
        )
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyLayoutComplexityStats {
    #[pyo3(get)]
    column_count: usize,
    #[pyo3(get)]
    ruled_table_count: usize,
    #[pyo3(get)]
    ruled_table_coverage: f32,
    #[pyo3(get)]
    text_table_run_count: usize,
    #[pyo3(get)]
    figure_count: usize,
    #[pyo3(get)]
    figure_coverage: f32,
    #[pyo3(get)]
    is_complex: bool,
    #[pyo3(get)]
    reasons: Vec<String>,
}

#[pymethods]
impl PyLayoutComplexityStats {
    fn __repr__(&self) -> String {
        format!(
            "LayoutComplexityStats(column_count={}, ruled_table_count={}, figure_coverage={:.2}, is_complex={})",
            self.column_count, self.ruled_table_count, self.figure_coverage, self.is_complex
        )
    }
}

impl PyLayoutComplexityStats {
    fn from_rust(stats: &liteparse::ocr_merge::LayoutComplexityStats) -> Self {
        Self {
            column_count: stats.column_count,
            ruled_table_count: stats.ruled_table_count,
            ruled_table_coverage: stats.ruled_table_coverage,
            text_table_run_count: stats.text_table_run_count,
            figure_count: stats.figure_count,
            figure_coverage: stats.figure_coverage,
            is_complex: stats.is_complex,
            reasons: stats
                .reasons
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
        }
    }
}

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyPageComplexityStats {
    #[pyo3(get)]
    page_number: usize,
    #[pyo3(get)]
    text_length: usize,
    #[pyo3(get)]
    text_coverage: f32,
    #[pyo3(get)]
    has_substantial_images: bool,
    /// Number of counted raster images — inline figures only; full-page
    /// backgrounds are excluded (see `full_page_image`).
    #[pyo3(get)]
    image_block_count: usize,
    /// Summed image-bbox area over page area, clamped to 1. Counts inline
    /// figures only: a full-page scan raster contributes 0 here — check
    /// `full_page_image` for that.
    #[pyo3(get)]
    image_coverage: f32,
    /// Largest single counted image's area over page area, clamped to 1. Same
    /// exclusion as `image_coverage`: a full-page raster contributes 0.
    #[pyo3(get)]
    largest_image_coverage: f32,
    /// A single raster covering ≥90% of the page is present. Such full-page
    /// backgrounds are excluded from `image_coverage`/`largest_image_coverage`
    /// (they're not inline figures), so this flag is the only signal that
    /// distinguishes a scan from a genuinely blank page — both otherwise
    /// report no text and no counted images.
    #[pyo3(get)]
    full_page_image: bool,
    #[pyo3(get)]
    uncovered_vector_area: Option<f32>,
    #[pyo3(get)]
    is_garbled: bool,
    #[pyo3(get)]
    page_area: f32,
    #[pyo3(get)]
    needs_ocr: bool,
    #[pyo3(get)]
    reasons: Vec<String>,
    #[pyo3(get)]
    layout: Option<PyLayoutComplexityStats>,
}

#[pymethods]
impl PyPageComplexityStats {
    fn __repr__(&self) -> String {
        format!(
            "PageComplexityStats(page_number={}, text_length={}, text_coverage={:.2}, needs_ocr={})",
            self.page_number, self.text_length, self.text_coverage, self.needs_ocr
        )
    }
}

impl PyPageComplexityStats {
    fn from_rust(stats: &liteparse::ocr_merge::PageComplexityStats) -> Self {
        Self {
            page_number: stats.page_number,
            text_length: stats.text_length,
            text_coverage: stats.text_coverage,
            has_substantial_images: stats.has_substantial_images,
            image_block_count: stats.image_block_count,
            image_coverage: stats.image_coverage,
            largest_image_coverage: stats.largest_image_coverage,
            full_page_image: stats.full_page_image,
            uncovered_vector_area: stats.uncovered_vector_area,
            is_garbled: stats.is_garbled,
            page_area: stats.page_area,
            needs_ocr: stats.needs_ocr,
            reasons: stats
                .reasons
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
            layout: stats
                .layout
                .as_ref()
                .map(PyLayoutComplexityStats::from_rust),
        }
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[pyclass(frozen, from_py_object)]
#[derive(Clone)]
struct PyLiteParseConfig {
    #[pyo3(get)]
    ocr_language: String,
    #[pyo3(get)]
    ocr_enabled: bool,
    #[pyo3(get)]
    ocr_server_url: Option<String>,
    #[pyo3(get)]
    ocr_server_headers: Option<HashMap<String, String>>,
    #[pyo3(get)]
    tessdata_path: Option<String>,
    #[pyo3(get)]
    max_pages: usize,
    #[pyo3(get)]
    target_pages: Option<String>,
    #[pyo3(get)]
    extract_screenshots: bool,
    #[pyo3(get)]
    continue_on_page_error: bool,
    #[pyo3(get)]
    dpi: f32,
    #[pyo3(get)]
    output_format: String,
    #[pyo3(get)]
    preserve_very_small_text: bool,
    #[pyo3(get)]
    password: Option<String>,
    #[pyo3(get)]
    quiet: bool,
    #[pyo3(get)]
    num_workers: usize,
    #[pyo3(get)]
    image_mode: String,
    #[pyo3(get)]
    extract_links: bool,
    #[pyo3(get)]
    keep_headers_footers: bool,
    #[pyo3(get)]
    extract_annotations: bool,
    #[pyo3(get)]
    extract_form_fields: bool,
    #[pyo3(get)]
    extract_structure_tree: bool,
    #[pyo3(get)]
    extract_blocks: bool,
    #[pyo3(get)]
    extract_xfa_packets: bool,
    #[pyo3(get)]
    extract_document_metadata: bool,
    #[pyo3(get)]
    extract_content_bounds: bool,
    #[pyo3(get)]
    detect_screenshot_rects: bool,
    #[pyo3(get)]
    render_form_fields: bool,
    #[pyo3(get)]
    ocr_failure_fatal: bool,
    #[pyo3(get)]
    ocr_hedge_delays_ms: Vec<u64>,
    #[pyo3(get)]
    emit_word_boxes: bool,
    #[pyo3(get)]
    crop_box: Option<(f32, f32, f32, f32)>,
    #[pyo3(get)]
    skip_diagonal_text: bool,
    #[pyo3(get)]
    include_complexity: bool,
    #[pyo3(get)]
    extract_text_metadata: bool,
    #[pyo3(get)]
    image_output_dir: Option<String>,
    #[pyo3(get)]
    extract_images: bool,
    #[pyo3(get)]
    extract_vector_graphics: bool,
}

#[pymethods]
impl PyLiteParseConfig {
    fn __repr__(&self) -> String {
        format!(
            "LiteParseConfig(ocr_enabled={}, dpi={}, max_pages={})",
            self.ocr_enabled, self.dpi, self.max_pages
        )
    }
}

impl PyLiteParseConfig {
    fn from_rust(cfg: &LiteParseConfig) -> Self {
        Self {
            ocr_language: cfg.ocr_language.clone(),
            ocr_enabled: cfg.ocr_enabled,
            ocr_server_url: cfg.ocr_server_url.clone(),
            ocr_server_headers: if cfg.ocr_server_headers.is_empty() {
                None
            } else {
                Some(cfg.ocr_server_headers.iter().cloned().collect())
            },
            tessdata_path: cfg.tessdata_path.clone(),
            max_pages: cfg.max_pages,
            target_pages: cfg.target_pages.clone(),
            extract_screenshots: cfg.extract_screenshots,
            continue_on_page_error: cfg.continue_on_page_error,
            dpi: cfg.dpi,
            output_format: match cfg.output_format {
                OutputFormat::Json => "json".to_string(),
                OutputFormat::Text => "text".to_string(),
                OutputFormat::Markdown => "markdown".to_string(),
            },
            preserve_very_small_text: cfg.preserve_very_small_text,
            password: cfg.password.clone(),
            quiet: cfg.quiet,
            num_workers: cfg.num_workers,
            image_mode: match cfg.image_mode {
                ImageMode::Off => "off".to_string(),
                ImageMode::Placeholder => "placeholder".to_string(),
                ImageMode::Embed => "embed".to_string(),
            },
            extract_links: cfg.extract_links,
            keep_headers_footers: cfg.keep_headers_footers,
            extract_annotations: cfg.extract_annotations,
            extract_form_fields: cfg.extract_form_fields,
            extract_structure_tree: cfg.extract_structure_tree,
            extract_blocks: cfg.extract_blocks,
            extract_xfa_packets: cfg.extract_xfa_packets,
            extract_document_metadata: cfg.extract_document_metadata,
            extract_content_bounds: cfg.extract_content_bounds,
            detect_screenshot_rects: cfg.detect_screenshot_rects,
            render_form_fields: cfg.render_form_fields,
            ocr_failure_fatal: cfg.ocr_failure_fatal,
            ocr_hedge_delays_ms: cfg.ocr_hedge_delays_ms.clone(),
            emit_word_boxes: cfg.emit_word_boxes,
            crop_box: cfg
                .crop_box
                .as_ref()
                .map(|c| (c.top, c.right, c.bottom, c.left)),
            skip_diagonal_text: cfg.skip_diagonal_text,
            include_complexity: cfg.include_complexity,
            extract_text_metadata: cfg.extract_text_metadata,
            image_output_dir: cfg.image_output_dir.clone(),
            extract_images: cfg.extract_images,
            extract_vector_graphics: cfg.extract_vector_graphics,
        }
    }
}

// ---------------------------------------------------------------------------
// Batch parsing
// ---------------------------------------------------------------------------

/// One batch of pages from a `_ParseSession`. Internal plumbing for the
/// wrapper's `parse_batches()`, which converts it into the public
/// `liteparse.types.ParseBatch` dataclass — the underscore name keeps the
/// two from colliding.
#[pyclass(frozen, name = "_ParseBatch", skip_from_py_object)]
#[derive(Clone)]
struct PyParseBatch {
    /// First source page in this batch (1-indexed).
    #[pyo3(get)]
    start_page: u32,
    /// Last source page in this batch (1-indexed, inclusive).
    #[pyo3(get)]
    end_page: u32,
    /// The pages in `start_page..=end_page`, as an ordinary parse result.
    #[pyo3(get)]
    result: PyParseResult,
}

/// A document opened once and parsed in bounded page batches. Internal
/// plumbing for the wrapper's `parse_batches()` — prefer that.
///
/// Iterate it directly to consume every batch:
///
///     for batch in parser.open_batch_session("large.pdf", batch_size=20):
///         handle(batch.result.pages)
#[pyclass(name = "_ParseSession", unsendable)]
struct PyParseSession {
    inner: liteparse::ParseSession,
    runtime: std::sync::Arc<tokio::runtime::Runtime>,
    extract_text_metadata: bool,
}

#[pymethods]
impl PyParseSession {
    /// Total pages in the source document, before `max_pages` or batching.
    #[getter]
    fn total_pages(&self) -> u32 {
        self.inner.total_pages()
    }

    /// Parse and return the next batch, or `None` once every page within
    /// `max_pages` has been yielded.
    fn next_batch(&mut self, py: Python<'_>) -> PyResult<Option<PyParseBatch>> {
        // Releasing the GIL keeps other Python threads running while PDFium
        // extraction and grid projection execute, matching `parse()`.
        let batch = py
            .detach(|| self.runtime.block_on(self.inner.next_batch()))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Ok(batch.map(|batch| PyParseBatch {
            start_page: batch.start_page,
            end_page: batch.end_page,
            result: PyParseResult::from_rust(batch.result, self.extract_text_metadata),
        }))
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Returning `None` raises `StopIteration`, ending the loop.
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<PyParseBatch>> {
        self.next_batch(py)
    }
}

// ---------------------------------------------------------------------------
// Main LiteParse class
// ---------------------------------------------------------------------------

#[pyclass]
struct LiteParse {
    inner: liteparse::parser::LiteParse,
    config: LiteParseConfig,
    runtime: std::sync::Arc<tokio::runtime::Runtime>,
}

#[pymethods]
impl LiteParse {
    #[new]
    #[pyo3(signature = (
        *,
        ocr_language = None,
        ocr_enabled = None,
        ocr_server_url = None,
        ocr_server_headers = None,
        tessdata_path = None,
        max_pages = None,
        target_pages = None,
        extract_screenshots = None,
        continue_on_page_error = None,
        dpi = None,
        output_format = None,
        preserve_very_small_text = None,
        password = None,
        quiet = None,
        num_workers = None,
        image_mode = None,
        extract_images = None,
        image_output_dir = None,
        extract_links = None,
        keep_headers_footers = None,
        extract_annotations = None,
        extract_form_fields = None,
        extract_structure_tree = None,
        extract_blocks = None,
        extract_xfa_packets = None,
        extract_document_metadata = None,
        extract_content_bounds = None,
        detect_screenshot_rects = None,
        render_form_fields = None,
        ocr_failure_fatal = None,
        ocr_hedge_delays_ms = None,
        emit_word_boxes = None,
        extract_text_metadata = None,
        crop_box = None,
        skip_diagonal_text = None,
        include_complexity = None,
        extract_vector_graphics = None,
    ))]
    fn new(
        ocr_language: Option<String>,
        ocr_enabled: Option<bool>,
        ocr_server_url: Option<String>,
        ocr_server_headers: Option<HashMap<String, String>>,
        tessdata_path: Option<String>,
        max_pages: Option<usize>,
        target_pages: Option<String>,
        extract_screenshots: Option<bool>,
        continue_on_page_error: Option<bool>,
        dpi: Option<f32>,
        output_format: Option<String>,
        preserve_very_small_text: Option<bool>,
        password: Option<String>,
        quiet: Option<bool>,
        num_workers: Option<usize>,
        image_mode: Option<String>,
        extract_images: Option<bool>,
        image_output_dir: Option<String>,
        extract_links: Option<bool>,
        keep_headers_footers: Option<bool>,
        extract_annotations: Option<bool>,
        extract_form_fields: Option<bool>,
        extract_structure_tree: Option<bool>,
        extract_blocks: Option<bool>,
        extract_xfa_packets: Option<bool>,
        extract_document_metadata: Option<bool>,
        extract_content_bounds: Option<bool>,
        detect_screenshot_rects: Option<bool>,
        render_form_fields: Option<bool>,
        ocr_failure_fatal: Option<bool>,
        ocr_hedge_delays_ms: Option<Vec<u64>>,
        emit_word_boxes: Option<bool>,
        extract_text_metadata: Option<bool>,
        crop_box: Option<(f32, f32, f32, f32)>,
        skip_diagonal_text: Option<bool>,
        include_complexity: Option<bool>,
        extract_vector_graphics: Option<bool>,
    ) -> PyResult<Self> {
        let mut cfg = LiteParseConfig::default();
        if let Some(v) = ocr_language {
            cfg.ocr_language = v;
        }
        if let Some(v) = ocr_enabled {
            cfg.ocr_enabled = v;
        }
        if let Some(v) = ocr_server_url {
            cfg.ocr_server_url = Some(v);
        }
        if let Some(v) = ocr_server_headers {
            cfg.ocr_server_headers = v.into_iter().collect();
        }
        if let Some(v) = tessdata_path {
            cfg.tessdata_path = Some(v);
        }
        if let Some(v) = max_pages {
            cfg.max_pages = v;
        }
        if let Some(v) = target_pages {
            cfg.target_pages = Some(v);
        }
        if let Some(v) = extract_screenshots {
            cfg.extract_screenshots = v;
        }
        if let Some(v) = continue_on_page_error {
            cfg.continue_on_page_error = v;
        }
        if let Some(v) = dpi {
            cfg.dpi = v;
        }
        if let Some(v) = output_format {
            cfg.output_format = match v.as_str() {
                "text" => OutputFormat::Text,
                "markdown" | "md" => OutputFormat::Markdown,
                _ => OutputFormat::Json,
            };
        }
        if let Some(v) = preserve_very_small_text {
            cfg.preserve_very_small_text = v;
        }
        if let Some(v) = password {
            cfg.password = Some(v);
        }
        if let Some(v) = quiet {
            cfg.quiet = v;
        }
        if let Some(v) = num_workers {
            cfg.num_workers = v;
        }
        if let Some(v) = image_mode {
            cfg.image_mode = match v.as_str() {
                "off" | "none" => ImageMode::Off,
                "embed" => ImageMode::Embed,
                _ => ImageMode::Placeholder,
            };
        }
        if let Some(v) = extract_images {
            cfg.extract_images = v;
        }
        if let Some(v) = image_output_dir {
            cfg.image_output_dir = Some(v);
        }
        if let Some(v) = extract_links {
            cfg.extract_links = v;
        }
        if let Some(v) = keep_headers_footers {
            cfg.keep_headers_footers = v;
        }
        if let Some(v) = extract_annotations {
            cfg.extract_annotations = v;
        }
        if let Some(v) = extract_form_fields {
            cfg.extract_form_fields = v;
        }
        if let Some(v) = extract_structure_tree {
            cfg.extract_structure_tree = v;
        }
        if let Some(v) = extract_blocks {
            cfg.extract_blocks = v;
        }
        if let Some(v) = extract_xfa_packets {
            cfg.extract_xfa_packets = v;
        }
        if let Some(v) = extract_document_metadata {
            cfg.extract_document_metadata = v;
        }
        if let Some(v) = extract_content_bounds {
            cfg.extract_content_bounds = v;
        }
        if let Some(v) = detect_screenshot_rects {
            cfg.detect_screenshot_rects = v;
        }
        if let Some(v) = render_form_fields {
            cfg.render_form_fields = v;
        }
        if let Some(v) = ocr_failure_fatal {
            cfg.ocr_failure_fatal = v;
        }
        if let Some(v) = ocr_hedge_delays_ms {
            cfg.ocr_hedge_delays_ms = v;
        }
        if let Some(v) = emit_word_boxes {
            cfg.emit_word_boxes = v;
        }
        if let Some(v) = extract_text_metadata {
            cfg.extract_text_metadata = v;
        }
        if let Some((top, right, bottom, left)) = crop_box {
            cfg.crop_box = Some(CropBox {
                top,
                right,
                bottom,
                left,
            });
        }
        if let Some(v) = skip_diagonal_text {
            cfg.skip_diagonal_text = v;
        }
        if let Some(v) = include_complexity {
            cfg.include_complexity = v;
        }
        if let Some(v) = extract_vector_graphics {
            cfg.extract_vector_graphics = v;
        }

        let inner = liteparse::parser::LiteParse::new(cfg.clone());
        let runtime = std::sync::Arc::new(
            tokio::runtime::Runtime::new()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?,
        );

        Ok(Self {
            inner,
            config: cfg,
            runtime,
        })
    }

    /// Parse a document from a file path.
    fn parse(&self, py: Python<'_>, input: String) -> PyResult<PyParseResult> {
        let pdf_input = PdfInput::Path(input);
        let result = py
            .detach(|| self.runtime.block_on(self.inner.parse_input(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(PyParseResult::from_rust(
            result,
            self.config.extract_text_metadata,
        ))
    }

    /// Open a document from a file path for bounded-memory batch parsing.
    /// Internal plumbing for the wrapper's `parse_batches()` — prefer that.
    ///
    /// Converts a non-PDF source once and returns a `_ParseSession` yielding
    /// `batch_size` pages at a time. Cross-page passes (repeated header/footer
    /// removal, image deduplication) see only the pages in their own batch, so
    /// output can differ from a whole-document `parse()`.
    #[pyo3(signature = (input, batch_size = None))]
    fn open_batch_session(
        &self,
        py: Python<'_>,
        input: String,
        batch_size: Option<usize>,
    ) -> PyResult<PyParseSession> {
        self.open_session(py, PdfInput::Path(input), batch_size)
    }

    /// Open a document from raw bytes for bounded-memory batch parsing.
    /// Internal plumbing for the wrapper's `parse_batches()` — prefer that.
    #[pyo3(signature = (data, batch_size = None))]
    fn open_batch_session_bytes(
        &self,
        py: Python<'_>,
        data: Vec<u8>,
        batch_size: Option<usize>,
    ) -> PyResult<PyParseSession> {
        self.open_session(py, PdfInput::Bytes(data), batch_size)
    }

    /// Parse a document from raw bytes.
    fn parse_bytes(&self, py: Python<'_>, data: Vec<u8>) -> PyResult<PyParseResult> {
        let pdf_input = PdfInput::Bytes(data);
        let result = py
            .detach(|| self.runtime.block_on(self.inner.parse_input(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(PyParseResult::from_rust(
            result,
            self.config.extract_text_metadata,
        ))
    }

    /// Determine per-page complexity for a document at the given path. Returns
    /// a list of PageComplexityStats — a cheap pre-OCR check with per-page
    /// signals and a `needs_ocr` verdict.
    fn is_complex(&self, py: Python<'_>, input: String) -> PyResult<Vec<PyPageComplexityStats>> {
        let pdf_input = PdfInput::Path(input);
        let stats = py
            .detach(|| self.runtime.block_on(self.inner.is_complex(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(stats.iter().map(PyPageComplexityStats::from_rust).collect())
    }

    /// Determine per-page complexity for a document from raw bytes.
    fn is_complex_bytes(
        &self,
        py: Python<'_>,
        data: Vec<u8>,
    ) -> PyResult<Vec<PyPageComplexityStats>> {
        let pdf_input = PdfInput::Bytes(data);
        let stats = py
            .detach(|| self.runtime.block_on(self.inner.is_complex(pdf_input)))
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(stats.iter().map(PyPageComplexityStats::from_rust).collect())
    }

    /// Take screenshots of document pages. Returns a list of ScreenshotResult.
    ///
    /// Non-PDF files are automatically converted to PDF before rendering when
    /// LibreOffice/ImageMagick are available.
    #[pyo3(signature = (input, page_numbers = None))]
    fn screenshot(
        &self,
        py: Python<'_>,
        input: String,
        page_numbers: Option<Vec<u32>>,
    ) -> PyResult<Vec<PyScreenshotResult>> {
        py.detach(|| {
            let results = self
                .runtime
                .block_on(self.inner.screenshot(&input, page_numbers))
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

            Ok(results
                .into_iter()
                .map(|r| PyScreenshotResult {
                    page_num: r.page_num,
                    width: r.width,
                    height: r.height,
                    image_buffer: r.image_bytes,
                    is_solid_fill: r.is_solid_fill,
                    rects: r
                        .rects
                        .into_iter()
                        .map(|rect| PyScreenshotRect {
                            x: rect.x as f64,
                            y: rect.y as f64,
                            width: rect.width as f64,
                            height: rect.height as f64,
                            color: rect.color,
                            is_line: rect.is_line,
                        })
                        .collect(),
                })
                .collect())
        })
    }

    /// Get the resolved configuration.
    #[getter]
    fn config(&self) -> PyLiteParseConfig {
        PyLiteParseConfig::from_rust(&self.config)
    }

    fn __repr__(&self) -> String {
        format!(
            "LiteParse(ocr_enabled={}, dpi={}, max_pages={})",
            self.config.ocr_enabled, self.config.dpi, self.config.max_pages
        )
    }
}

impl LiteParse {
    /// Shared body of `open_batch_session` / `open_batch_session_bytes`. Not
    /// a `#[pymethods]` entry, so it stays off the Python surface.
    fn open_session(
        &self,
        py: Python<'_>,
        input: PdfInput,
        batch_size: Option<usize>,
    ) -> PyResult<PyParseSession> {
        let batch_size = batch_size.unwrap_or(liteparse::DEFAULT_PAGE_BATCH_SIZE);
        let session = py
            .detach(|| {
                self.runtime
                    .block_on(self.inner.open_batch_session(input, batch_size))
            })
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Ok(PyParseSession {
            inner: session,
            runtime: self.runtime.clone(),
            extract_text_metadata: self.config.extract_text_metadata,
        })
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

/// Duck-typed input for [`search_items`]. Extracts by attribute name, so it
/// accepts both the native `PyTextItem` and the pure-Python `TextItem`
/// dataclass that `LiteParse.parse()` hands back (see `parser.py`,
/// `_convert_native_result`).
#[derive(FromPyObject)]
struct SearchInputItem {
    text: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[pyo3(default)]
    font_name: Option<String>,
    #[pyo3(default)]
    font_size: Option<f64>,
    #[pyo3(default)]
    confidence: Option<f64>,
    #[pyo3(default)]
    rotation: f64,
}

impl SearchInputItem {
    fn to_rust(&self) -> liteparse::types::TextItem {
        liteparse::types::TextItem {
            text: self.text.clone(),
            x: self.x as f32,
            y: self.y as f32,
            width: self.width as f32,
            height: self.height as f32,
            rotation: self.rotation as f32,
            font_name: self.font_name.clone(),
            font_size: self.font_size.map(|v| v as f32),
            confidence: self.confidence.map(|v| v as f32),
            ..Default::default()
        }
    }
}

/// Search text items for phrase matches, returning merged items with combined bounding boxes.
#[pyfunction]
#[pyo3(signature = (items, phrase, *, case_sensitive = false))]
fn search_items(
    items: Vec<SearchInputItem>,
    phrase: String,
    case_sensitive: bool,
) -> Vec<PyTextItem> {
    let rust_items: Vec<_> = items.iter().map(|i| i.to_rust()).collect();
    let options = liteparse::search::SearchOptions {
        phrase,
        case_sensitive,
    };
    liteparse::search::search_items(&rust_items, &options)
        .into_iter()
        .map(PyTextItem::from_rust)
        .collect()
}

/// Run the `lit` CLI with the given arguments.
#[pyfunction]
fn run_cli(args: Vec<String>) -> PyResult<()> {
    cli::run_cli(args).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pymodule]
fn _liteparse(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<LiteParse>()?;
    m.add_class::<PyLiteParseConfig>()?;
    m.add_class::<PyParseResult>()?;
    m.add_class::<PyPageError>()?;
    m.add_class::<PyParseBatch>()?;
    m.add_class::<PyParseSession>()?;
    m.add_class::<PyDocumentMetadata>()?;
    m.add_class::<PyExtractedImage>()?;
    m.add_class::<PyImageRect>()?;
    m.add_class::<PyParsedPage>()?;
    m.add_class::<PyTextItem>()?;
    m.add_class::<PyWordBox>()?;
    m.add_class::<PyAnnotationRect>()?;
    m.add_class::<PyDocumentAnnotation>()?;
    m.add_class::<PyStructureAttribute>()?;
    m.add_class::<PyStructureTreeElement>()?;
    m.add_class::<PyStructureTree>()?;
    m.add_class::<PyLayoutCell>()?;
    m.add_class::<PyLayoutBlock>()?;
    m.add_class::<PyFormField>()?;
    m.add_class::<PyScreenshotResult>()?;
    m.add_class::<PyScreenshotRect>()?;
    m.add_class::<PyXfaPacket>()?;
    m.add_class::<PyPageComplexityStats>()?;
    m.add_class::<PyLayoutComplexityStats>()?;
    m.add_class::<PyRect>()?;
    m.add_class::<PyVectorShape>()?;
    m.add_class::<PyVectorLine>()?;
    m.add_class::<PyVectorGraphics>()?;
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add_function(wrap_pyfunction!(search_items, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_metadata_round_trips_through_python_type() {
        let item = liteparse::types::TextItem {
            text: "A".into(),
            font_height: Some(12.0),
            font_ascent: Some(9.0),
            font_descent: Some(-3.0),
            font_weight: Some(700),
            text_width: Some(8.0),
            font_is_buggy: true,
            mcid: Some(2),
            fill_color: Some("ff112233".into()),
            stroke_color: Some("ff445566".into()),
            char_codes: vec![65, 32],
            trailing_space_generated: true,
            ..Default::default()
        };

        let py = PyTextItem::from_rust(item);
        assert_eq!(py.char_codes, vec![65, 32]);
        assert!(py.trailing_space_generated);
        assert_eq!(py.fill_color.as_deref(), Some("ff112233"));

        let round_trip = py.to_rust();
        assert_eq!(round_trip.font_height, Some(12.0));
        assert_eq!(round_trip.font_ascent, Some(9.0));
        assert_eq!(round_trip.font_descent, Some(-3.0));
        assert_eq!(round_trip.font_weight, Some(700));
        assert_eq!(round_trip.text_width, Some(8.0));
        assert!(round_trip.font_is_buggy);
        assert_eq!(round_trip.mcid, Some(2));
        assert_eq!(round_trip.stroke_color.as_deref(), Some("ff445566"));
        assert_eq!(round_trip.char_codes, vec![65, 32]);
        assert!(round_trip.trailing_space_generated);
    }

    #[test]
    fn text_metadata_config_defaults_off_and_can_be_enabled() {
        let py = PyLiteParseConfig::from_rust(&LiteParseConfig::default());
        assert!(!py.extract_text_metadata);

        let config = LiteParseConfig {
            extract_text_metadata: true,
            ..Default::default()
        };
        let py = PyLiteParseConfig::from_rust(&config);
        assert!(py.extract_text_metadata);
    }

    #[test]
    fn converts_vector_graphics_to_python_shape() {
        let rust = liteparse::types::VectorGraphics {
            shapes: vec![liteparse::types::VectorShape {
                bbox: liteparse::types::Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
                stroke: false,
                stroke_color: None,
                fill: true,
                fill_color: Some("ffffffff".into()),
                has_curve: false,
            }],
            lines: vec![],
        };
        let py = PyVectorGraphics::from_rust(rust);
        assert_eq!(py.shapes[0].bbox.height, 4.0);
        assert_eq!(py.shapes[0].fill_color.as_deref(), Some("ffffffff"));
    }
}
