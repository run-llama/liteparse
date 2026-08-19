use crate::bidi::is_rtl_char;
use crate::error::LiteParseError;
use crate::glyph_names::resolve_glyph_name;
use crate::types::{
    DocumentAnnotation, ExtractedImage, FormField, GraphicPrimitive, ImageRef, OutlineTarget,
    Page as LitePage, PageError, PdfInput, Rect, StructNode, StructureAttributeValue,
    StructureTree, StructureTreeElement, TextItem, VectorGraphics, VectorLine, VectorShape,
    WordBox,
};
use image::ImageEncoder;
use pdfium::{
    Document, Font, FontType, FormEnvironment, Library, Page, PathObject, PdfLink, RectF,
    SegmentKind, TextPage,
};

/// Open a PDF from path or bytes with an optional password.
///
/// The returned [`Document`] borrows from the provided [`Library`], which
/// holds the process-global PDFium lock. The lock is released when the
/// `Library` is dropped, so callers must keep `lib` alive for as long as any
/// `Document` / `Page` / `TextPage` etc. derived from it is in use.
pub(crate) fn load_document_from_input<'lib>(
    lib: &'lib Library,
    input: &PdfInput,
    password: Option<&str>,
) -> Result<Document<'lib>, LiteParseError> {
    match input {
        PdfInput::Path(path) => Ok(lib.load_document(path, password)?),
        PdfInput::Bytes(data) => Ok(lib.load_document_from_bytes(data, password)?),
    }
}

/// Extract pages from a `PdfInput` (file path or bytes) with filtering.
///
/// This convenience entry point acquires the PDFium lock internally for the
/// full extraction. Callers that already hold a [`Library`] (e.g. because
/// they're also rendering bitmaps in the same critical section) should call
/// [`extract_pages_from_document`] directly.
pub fn extract_pages_from_input(
    input: &PdfInput,
    target_pages: Option<&[u32]>,
    max_pages: usize,
    password: Option<&str>,
) -> Result<Vec<LitePage>, LiteParseError> {
    let lib = Library::init();
    let document = load_document_from_input(&lib, input, password)?;
    extract_pages_from_document(&document, target_pages, max_pages)
}

/// Extract pages from an already-open PDFium document.
pub(crate) fn extract_pages_from_document(
    document: &Document,
    target_pages: Option<&[u32]>,
    max_pages: usize,
) -> Result<Vec<LitePage>, LiteParseError> {
    Ok(extract_pages_and_images(
        document,
        target_pages,
        max_pages,
        false,
        None,
        ExtractionOutputOptions::default(),
    )?
    .pages)
}

/// Output of [`extract_pages_and_images`].
pub(crate) struct ExtractedPages {
    pub pages: Vec<LitePage>,
    pub page_errors: Vec<PageError>,
    /// Empty unless `output_options.extract_images` was set.
    pub images: Vec<ExtractedImage>,
    pub image_error_count: u32,
    /// Whether any page was flattened to recover form-widget text. Flattening
    /// mutates the open PDFium document, so a caller that still needs the
    /// original widget annotations must reopen the input.
    pub flattened_form_widgets: bool,
}

/// Same as `extract_pages_from_document` but optionally also renders every
/// raster image object to bytes (when `output_options.extract_images` is true). Returned
/// `ExtractedImage`s carry the same ids the markdown emitter will reference,
/// so callers can match them up by id.
pub(crate) fn extract_pages_and_images(
    document: &Document,
    target_pages: Option<&[u32]>,
    max_pages: usize,
    extract_links: bool,
    glyph_resolver: Option<&dyn crate::GlyphResolver>,
    output_options: ExtractionOutputOptions,
) -> Result<ExtractedPages, LiteParseError> {
    let page_count = document.page_count();
    let mut pages = Vec::new();
    let mut page_errors = Vec::new();
    let mut images: Vec<ExtractedImage> = Vec::new();
    let mut image_cache = ImageCache::default();
    let mut image_error_count = 0u32;
    let mut flattened_form_widgets = false;
    // One FFI call keeps the per-page annotation walk off the hot path for
    // every document without an AcroForm catalog, which is nearly all of them.
    let document_has_form = document.form_type() != 0;
    let form_environment = output_options
        .extract_form_fields
        .then(|| document.form_environment())
        .flatten();

    for page_index in 0..page_count {
        let page_number = page_index as u32 + 1;

        if let Some(targets) = target_pages
            && !targets.contains(&page_number)
        {
            continue;
        }

        if pages.len() + page_errors.len() >= max_pages {
            break;
        }

        let page_result = extract_single_page(
            document,
            page_index,
            page_number,
            extract_links,
            glyph_resolver,
            &output_options,
            form_environment.as_ref(),
            document_has_form,
            &mut image_cache,
        );

        match resolve_page_result(
            page_number,
            page_result,
            output_options.continue_on_page_error,
            &mut page_errors,
        )? {
            Some(extraction) => {
                pages.push(extraction.page);
                images.extend(extraction.images);
                image_error_count += extraction.image_error_count;
                flattened_form_widgets |= extraction.flattened_form_widgets;
            }
            // A failed page's cached renders must not seed dedup for later
            // pages: a hit would emit `duplicate_of` pointing at an image id
            // that never made it into the output.
            None => image_cache.remove_page(page_number),
        }
    }

    Ok(ExtractedPages {
        pages,
        page_errors,
        images,
        image_error_count,
        flattened_form_widgets,
    })
}

/// Everything one successfully extracted page contributes to the document
/// output. Accumulated page-locally so a failed page is rolled back by
/// dropping this value (plus [`ImageCache::remove_page`] for its cache
/// inserts) instead of undoing shared-state mutations.
struct PageExtraction {
    page: LitePage,
    /// Rendered image bytes; empty unless `extract_images` was set.
    images: Vec<ExtractedImage>,
    image_error_count: u32,
    /// Whether this page was flattened to recover form-widget text.
    flattened_form_widgets: bool,
}

#[allow(clippy::too_many_arguments)]
fn extract_single_page(
    document: &Document,
    page_index: i32,
    page_number: u32,
    extract_links: bool,
    glyph_resolver: Option<&dyn crate::GlyphResolver>,
    output_options: &ExtractionOutputOptions,
    form_environment: Option<&FormEnvironment<'_, '_>>,
    document_has_form: bool,
    image_cache: &mut ImageCache,
) -> Result<PageExtraction, LiteParseError> {
    let mut images = Vec::new();
    let mut image_error_count = 0u32;
    let mut flattened_form_widgets = false;
    let page = document.page(page_index)?;
    let raw_page_width = page.width();
    let raw_page_height = page.height();
    let view_box = page.view_box().unwrap_or(RectF {
        left: 0.0,
        top: raw_page_height,
        right: raw_page_width,
        bottom: 0.0,
    });
    // All extracted geometry is converted to the rotation-adjusted
    // viewport coordinate space. Keep the page dimensions in that same
    // space so projection, filtering, and consumers do not clip content
    // at the unrotated MediaBox width on /Rotate 90 or /Rotate 270 pages.
    let (page_width, page_height) = page.viewport_size(&view_box);
    // Once a qualifying widget is found, PDFium flattens every visible
    // annotation on the page. Collect every annotation-backed output first.
    let links = if extract_links {
        page.links(&view_box)
    } else {
        Vec::new()
    };
    // Computed when emitted (`extract_content_bounds`) or needed
    // internally by the white-fill heuristic (`extract_vector_graphics`).
    let content_bounds = (output_options.extract_content_bounds
        || output_options.extract_vector_graphics)
        .then(|| {
            page.content_bounds()
                .map(|bounds| rect_from_pdfium(page.bounds_to_viewport(&view_box, &bounds)))
        })
        .flatten();
    let paths = page.path_objects(&view_box);
    let graphics = extract_layout_graphics(&paths);
    let vector_graphics = output_options
        .extract_vector_graphics
        .then(|| build_vector_graphics(&paths, content_bounds.as_ref()));
    let struct_nodes = extract_page_struct_nodes(&page, &view_box);
    let extracted_refs = extract_page_image_refs(&page, page_number, output_options.extract_images);
    let mut image_refs = extracted_refs.refs;
    image_error_count += extracted_refs.error_count;
    let pdf_annotations = (output_options.extract_annotations
        || output_options.extract_structure_tree)
        .then(|| page.annotations(&view_box))
        .unwrap_or_default();
    let annotations = output_options
        .extract_annotations
        .then(|| pdf_annotations.iter().map(document_annotation).collect());
    let structure_tree = output_options.extract_structure_tree.then(|| {
        let annotations_by_object = pdf_annotations
            .iter()
            .filter(|annotation| annotation.subtype == "link")
            .filter_map(|annotation| annotation.object_number.map(|n| (n, annotation)))
            .collect::<std::collections::HashMap<_, _>>();
        StructureTree {
            roots: page
                .structure_tree()
                .into_iter()
                .map(|element| structure_tree_element(element, &annotations_by_object))
                .collect(),
        }
    });
    let form_fields = output_options.extract_form_fields.then(|| {
        form_environment.map_or_else(Vec::new, |form| {
            page.form_fields(form, &view_box, page_number)
                .into_iter()
                .map(|field| FormField {
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
                    rect: field.rect.map(rect_from_pdfium),
                    options: field.options,
                    selected_options: field.selected_options,
                })
                .collect()
        })
    });

    if output_options.extract_images && !image_refs.is_empty() {
        let rendered = render_page_images(&page, page_number, &image_refs, image_cache);
        image_error_count += rendered.error_count;
        images.extend(rendered.images);
        for image_ref in &mut image_refs {
            image_ref.jpeg_bytes = None;
            image_ref.raw_bytes = None;
        }
    }

    // PDFium's text API reads only the page content stream. Filled form
    // values commonly live in widget appearance streams, so promote only
    // those widget appearances into page content and reload before text
    // extraction. Non-widget annotations are excluded, and this does not
    // initialize the form environment or execute document JS.
    //
    // `widget_text_rects` is empty for the overwhelming majority of pages —
    // documents with no AcroForm catalog never even reach the annotation
    // walk — so the whole path costs one `form_type()` call for most files.
    let extract_text = |page: &Page| -> Result<Vec<TextItem>, LiteParseError> {
        let text_page = page.text()?;
        extract_page_text_items(
            page,
            &text_page,
            &view_box,
            glyph_resolver,
            output_options.emit_word_boxes,
            output_options.extract_text_metadata,
        )
    };
    let widget_text_rects = if document_has_form {
        page.form_widget_text_rects(&view_box)
    } else {
        Vec::new()
    };
    let mut text_items = if widget_text_rects.is_empty() {
        extract_text(&page)?
    } else {
        // PDFium's text layer keeps only one of two runs that start at
        // essentially the same point, so a flattened appearance can
        // suppress page text it lands on. Usually widget rects sit over
        // blank space, and this bounds-only probe says so without touching
        // the text API; only when a widget really does cover existing text
        // do we extract twice and put back what was suppressed.
        let overlaps_existing_text = page.text_objects_overlap(&view_box, &widget_text_rects);
        let before = overlaps_existing_text
            .then(|| extract_text(&page))
            .transpose()?;
        drop(page);
        match document.flatten_form_widgets(page_index)? {
            Some(flattened_page) => {
                flattened_form_widgets = true;
                let mut items = extract_text(&flattened_page)?;
                if let Some(before) = before {
                    restore_flattened_over_text(&mut items, before, &widget_text_rects);
                }
                items
            }
            None => extract_text(&document.page(page_index)?)?,
        }
    };
    assign_links(&mut text_items, &links);
    assign_strikethrough(&mut text_items, &graphics);

    Ok(PageExtraction {
        page: LitePage {
            page_number: page_number as usize,
            page_width,
            page_height,
            content_bounds: output_options
                .extract_content_bounds
                .then_some(content_bounds)
                .flatten(),
            text_items,
            graphics,
            vector_graphics,
            struct_nodes,
            image_refs,
            annotations,
            form_fields,
            structure_tree,
        },
        images,
        image_error_count,
        flattened_form_widgets,
    })
}

fn resolve_page_result<T>(
    page_number: u32,
    result: Result<T, LiteParseError>,
    continue_on_page_error: bool,
    page_errors: &mut Vec<PageError>,
) -> Result<Option<T>, LiteParseError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if continue_on_page_error => {
            page_errors.push(PageError {
                page_number,
                message: error.to_string(),
            });
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Put back page text that flattening suppressed.
///
/// PDFium's text layer emits only one of two text runs that start at
/// essentially the same point, so a flattened widget appearance can knock out
/// page text it lands on. When the two carry the *same* string — a producer
/// that wrote the value into both the content stream and the appearance — that
/// is precisely the dedup a partially flattened file needs, and matching on
/// trimmed text leaves it alone. When they differ, such as a pre-printed label
/// sitting where the value is typed, dropping one is pure data loss, so the
/// pre-flatten copy is restored.
///
/// Only called for the page where a widget rect actually covers existing text;
/// `before` is the pre-flatten extraction of the same page.
///
/// Note this recovers one direction only. If the collision goes the other way
/// PDFium can suppress the *appearance* text instead, and the field value is
/// lost with no pre-flatten copy to restore it from.
fn restore_flattened_over_text(
    items: &mut Vec<TextItem>,
    before: Vec<TextItem>,
    widget_rects: &[RectF],
) {
    let surviving: std::collections::HashSet<&str> = items
        .iter()
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .collect();
    let mut restored: Vec<TextItem> = before
        .iter()
        .filter(|item| {
            let text = item.text.trim();
            !text.is_empty()
                && !surviving.contains(text)
                && widget_rects
                    .iter()
                    .any(|rect| rect_contains_center(rect, item))
        })
        .cloned()
        .collect();
    items.append(&mut restored);
}

fn rect_contains_center(rect: &RectF, item: &TextItem) -> bool {
    let cx = item.x + item.width / 2.0;
    let cy = item.y + item.height / 2.0;
    cx >= rect.left && cx <= rect.right && cy >= rect.top && cy <= rect.bottom
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExtractionOutputOptions {
    pub continue_on_page_error: bool,
    pub extract_content_bounds: bool,
    pub extract_text_metadata: bool,
    pub extract_images: bool,
    pub extract_vector_graphics: bool,
    pub extract_annotations: bool,
    pub extract_form_fields: bool,
    pub extract_structure_tree: bool,
    pub emit_word_boxes: bool,
}

fn document_annotation(annotation: &pdfium::PdfAnnotation) -> DocumentAnnotation {
    DocumentAnnotation {
        subtype: annotation.subtype.clone(),
        contents: annotation.contents.clone(),
        created: annotation.created.clone(),
        modified: annotation.modified.clone(),
        title: annotation.title.clone(),
        rect: annotation.rect.map(rect_from_pdfium),
        quadpoint_rects: annotation
            .quadpoint_rects
            .iter()
            .copied()
            .map(rect_from_pdfium)
            .collect(),
        uri: annotation.uri.clone(),
    }
}

fn structure_tree_element(
    element: pdfium::StructureElement,
    annotations_by_object: &std::collections::HashMap<i32, &pdfium::PdfAnnotation>,
) -> StructureTreeElement {
    let attributes = element
        .attributes
        .into_iter()
        .map(|(name, value)| {
            let value = match value {
                pdfium::StructureAttributeValue::Boolean(v) => StructureAttributeValue::Boolean(v),
                pdfium::StructureAttributeValue::Number(v) => StructureAttributeValue::Number(v),
                pdfium::StructureAttributeValue::String(v) => StructureAttributeValue::String(v),
            };
            (name, value)
        })
        .collect();
    StructureTreeElement {
        element_type: element.element_type,
        id: element.id,
        actual_text: element.actual_text,
        alt_text: element.alt_text,
        title: element.title,
        attributes,
        marked_content_ids: element.marked_content_ids,
        children: element
            .children
            .into_iter()
            .map(|child| structure_tree_element(child, annotations_by_object))
            .collect(),
        annotations: element
            .annotation_object_numbers
            .into_iter()
            .filter_map(|number| annotations_by_object.get(&number))
            .map(|annotation| document_annotation(annotation))
            .collect(),
    }
}

fn rect_from_pdfium(rect: RectF) -> Rect {
    Rect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    }
}

/// Assign hyperlink URIs to text items whose bbox center falls inside a link
/// annotation's rectangle. Both the item bbox and the link rect are in
/// viewport space. First matching link wins.
///
/// A link rect taller than `MULTILINE_DROP_FACTOR`× the height of the text it
/// covers is a multi-line annotation given to us as a single *union* box (no
/// per-line quad points). Its true anchor — which words on the intervening
/// lines are actually linked — is unrecoverable, so we drop it rather than
/// wrap a whole sentence in a misleading link. Well-formed multi-line links
/// expose quad points and arrive here as one single-line rect per line.
fn assign_links(items: &mut [TextItem], links: &[PdfLink]) {
    if links.is_empty() {
        return;
    }
    const MULTILINE_DROP_FACTOR: f32 = 1.8;
    for link in links {
        let r = &link.rect;
        let covered: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                let cx = it.x + it.width / 2.0;
                let cy = it.y + it.height / 2.0;
                cx >= r.left && cx <= r.right && cy >= r.top && cy <= r.bottom
            })
            .map(|(i, _)| i)
            .collect();
        if covered.is_empty() {
            continue;
        }
        let mut heights: Vec<f32> = covered.iter().map(|&i| items[i].height).collect();
        heights.sort_by(f32::total_cmp);
        let median_h = heights[heights.len() / 2];
        if median_h > 0.0 && (r.bottom - r.top) > MULTILINE_DROP_FACTOR * median_h {
            continue;
        }
        for &i in &covered {
            if items[i].link.is_none() {
                items[i].link = Some(link.uri.clone());
            }
        }
    }
}

/// Max thickness (pt) for a stroke/rect to count as a strikethrough line.
const STRIKE_MAX_THICKNESS_PT: f32 = 2.0;
/// A strike line must horizontally cover at least this fraction of the item.
const STRIKE_MIN_COVER_FRACTION: f32 = 0.6;

/// Mark text items whose vertical *middle* band is crossed by a thin horizontal
/// line (a strikethrough). The line may be drawn as a `Stroke` or as a thin
/// filled `Rect`. Underlines (near the baseline) and overlines (near the top)
/// are excluded by the band check; table rules / HRs almost never pass through
/// the middle of a glyph run, and the per-item width-coverage gate keeps long
/// dividers from tagging incidental text they happen to cross.
fn assign_strikethrough(items: &mut [TextItem], graphics: &[GraphicPrimitive]) {
    // Reduce graphics to horizontal segments: (xmin, xmax, y_center).
    let mut segs: Vec<(f32, f32, f32)> = Vec::new();
    for g in graphics {
        match g {
            GraphicPrimitive::Stroke {
                x1,
                y1,
                x2,
                y2,
                width,
                ..
            } => {
                let dy = (y1 - y2).abs();
                let dx = (x1 - x2).abs();
                if dy <= STRIKE_MAX_THICKNESS_PT && *width <= STRIKE_MAX_THICKNESS_PT && dx > dy {
                    segs.push((x1.min(*x2), x1.max(*x2), (y1 + y2) * 0.5));
                }
            }
            GraphicPrimitive::Rect { bbox, .. } => {
                // A thin, wide filled rect acts as a line.
                if bbox.height <= STRIKE_MAX_THICKNESS_PT && bbox.width > bbox.height {
                    segs.push((bbox.x, bbox.x + bbox.width, bbox.y + bbox.height * 0.5));
                }
            }
        }
    }
    if segs.is_empty() {
        return;
    }

    for item in items.iter_mut() {
        if item.width <= 0.0 || item.height <= 0.0 || item.text.trim().is_empty() {
            continue;
        }
        // Viewport space is top-left origin, so `y` is the top edge. The middle
        // band sits below the top and above the baseline, excluding over/underlines.
        let band_top = item.y + item.height * 0.20;
        let band_bot = item.y + item.height * 0.65;
        let (ix0, ix1) = (item.x, item.x + item.width);
        for &(sx0, sx1, sy) in &segs {
            if sy < band_top || sy > band_bot {
                continue;
            }
            let overlap = (ix1.min(sx1) - ix0.max(sx0)).max(0.0);
            if overlap >= item.width * STRIKE_MIN_COVER_FRACTION {
                item.strike = true;
                break;
            }
        }
    }
}

/// Walk the document outline (bookmarks). Returns entries in pre-order.
/// Empty when the PDF has no outline.
pub(crate) fn extract_outline(document: &Document) -> Vec<OutlineTarget> {
    document
        .outline()
        .into_iter()
        .filter_map(|e| {
            Some(OutlineTarget {
                level: e.level,
                title: e.title,
                page_index: e.page_index?,
                y_pdf: e.y,
            })
        })
        .collect()
}

/// Walk the page's structure tree (tagged PDFs). Returns nodes in pre-order;
/// empty when the page is untagged.
fn extract_page_struct_nodes(page: &Page, view_box: &RectF) -> Vec<StructNode> {
    page.struct_tree(view_box)
        .into_iter()
        .map(|n| StructNode {
            role: n.role,
            mcids: n.mcids,
            bbox: n.bbox.map(|b| Rect {
                x: b.left,
                y: b.top,
                width: b.right - b.left,
                height: b.bottom - b.top,
            }),
            alt_text: n.alt_text,
        })
        .collect()
}

/// Extract raw text items and print each page as a JSON-line object to stdout.
pub fn extract(pdf_path: &str, page_num: Option<u32>) -> Result<(), LiteParseError> {
    let target_pages: Option<Vec<u32>> = page_num.map(|p| vec![p]);
    let pages = extract_pages_from_input(
        &PdfInput::Path(pdf_path.to_string()),
        target_pages.as_deref(),
        usize::MAX,
        None,
    )?;
    for page in &pages {
        println!("{}", serde_json::to_string(page)?);
    }
    Ok(())
}

/// Check if the page has any visible (non-render-mode-3) printable characters.
/// Used to decide whether to skip invisible text or use it (OCR text layers).
/// Determine whether invisible (render mode 3) characters should be skipped.
///
/// Returns true only when the page has a clear mix of visible and invisible
/// text with the visible portion dominating — this indicates the invisible
/// text is likely a redundant OCR layer over a native-text PDF.
///
/// When invisible text is the majority, or the only text on the page,
/// returns false so we keep it (it IS the content, e.g. scanned PDFs with
/// an OCR text layer and no native text).
fn should_skip_invisible(text_page: &TextPage, char_count: i32) -> bool {
    let mut visible = 0u32;
    let mut invisible = 0u32;

    for i in 0..char_count {
        let Some(ch) = text_page.char_at(i) else {
            continue;
        };
        let unicode = ch.unicode();
        if unicode == 0 || unicode == 0xFFFE || unicode == 0xFFFF {
            continue;
        }
        if let Some(c) = char::from_u32(unicode)
            && (c.is_whitespace() || c.is_control())
        {
            continue;
        }
        if ch.is_generated() {
            continue;
        }
        if ch.text_render_mode() == Some(3) {
            invisible += 1;
        } else {
            visible += 1;
        }
    }

    // Only skip invisible text when visible text clearly dominates.
    // If invisible text is a significant portion (>30% of all text),
    // keep it — the page likely has mixed content where both matter.
    if visible == 0 {
        return false; // All invisible → keep it
    }
    if invisible == 0 {
        return false; // No invisible text to skip
    }
    let total = visible + invisible;
    let invisible_ratio = invisible as f64 / total as f64;
    invisible_ratio < 0.3
}

/// Minimum image extent (in PDF points) below which we ignore the image
/// object. Filters out hairline rasterized rules, icons embedded in glyphs,
/// and other sub-25pt fragments that would otherwise pollute the figure
/// stream. Matches the threshold used by `ocr_merge::has_images`.
const IMAGE_MIN_SIZE_PT: f32 = 25.0;

/// Max fraction of the page each axis can cover. Drops full-page background
/// images (scanned pages, watermarks).
const IMAGE_MAX_COVERAGE: f32 = 0.9;

/// Extract every image referenced in `refs`, preserving valid JPEG streams and
/// rendering other images to PNG. Returns one `ExtractedImage` per ref. Used
/// when embedded-image extraction is explicitly enabled. Failures for
/// individual images are counted but do not fail the whole parse.
struct CachedImage {
    raw_bytes: Vec<u8>,
    id: String,
    /// Source page of the canonical render; lets a failed page's inserts be
    /// rolled back so later duplicates never reference a dropped image id.
    page: u32,
    format: String,
    bytes: std::sync::Arc<Vec<u8>>,
}

/// Dedup key: hash of the source stream plus the metadata that affects
/// decoding. The hash only prefilters; the full raw bytes are still compared
/// on lookup within the matching bucket.
#[derive(PartialEq, Eq, Hash)]
struct CacheKey {
    raw_hash: u64,
    width: u32,
    height: u32,
    bits_per_pixel: u32,
    colorspace: i32,
}

#[derive(Default)]
pub(crate) struct ImageCache {
    entries: std::collections::HashMap<CacheKey, Vec<CachedImage>>,
}

impl ImageCache {
    fn key(r: &ImageRef, raw_bytes: &[u8]) -> CacheKey {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        raw_bytes.hash(&mut hasher);
        CacheKey {
            raw_hash: hasher.finish(),
            width: r.pixel_width,
            height: r.pixel_height,
            bits_per_pixel: r.bits_per_pixel,
            colorspace: r.colorspace,
        }
    }

    fn get(&self, r: &ImageRef, raw_bytes: &[u8]) -> Option<&CachedImage> {
        self.entries
            .get(&Self::key(r, raw_bytes))?
            .iter()
            .find(|entry| entry.raw_bytes == *raw_bytes)
    }

    fn insert(&mut self, r: &ImageRef, entry: CachedImage) {
        self.entries
            .entry(Self::key(r, &entry.raw_bytes))
            .or_default()
            .push(entry);
    }

    /// Drop every entry rendered from `page_number`. Called when that page
    /// fails after its images were cached, so a later duplicate can't resolve
    /// to a canonical image that was rolled back out of the output.
    fn remove_page(&mut self, page_number: u32) {
        self.entries.retain(|_, bucket| {
            bucket.retain(|entry| entry.page != page_number);
            !bucket.is_empty()
        });
    }
}

pub(crate) struct RenderedImages {
    pub images: Vec<ExtractedImage>,
    pub error_count: u32,
}

fn render_page_images(
    page: &Page,
    page_number: u32,
    refs: &[ImageRef],
    cache: &mut ImageCache,
) -> RenderedImages {
    let mut out = Vec::with_capacity(refs.len());
    let mut error_count = 0;
    for r in refs {
        if let Some(raw_bytes) = r.raw_bytes.as_ref()
            && let Some(cached) = cache.get(r, raw_bytes)
        {
            out.push(ExtractedImage {
                id: r.id.clone(),
                name: format!("img_{}.{}", r.id, cached.format),
                path: None,
                page: page_number,
                bbox: r.bbox.clone(),
                width: r.pixel_width,
                height: r.pixel_height,
                rotation: r.rotation,
                format: cached.format.clone(),
                duplicate_of: Some(cached.id.clone()),
                bytes: std::sync::Arc::clone(&cached.bytes),
            });
            continue;
        }

        let encoded = if let Some(jpeg) = r.jpeg_bytes.clone() {
            Ok(("jpg".to_string(), jpeg))
        } else {
            let bmp = match page.render_image_object(r.obj_index) {
                Ok(b) => b,
                Err(_) => {
                    error_count += 1;
                    continue;
                }
            };
            let w = bmp.width().max(0) as u32;
            let h = bmp.height().max(0) as u32;
            if w == 0 || h == 0 {
                error_count += 1;
                continue;
            }
            let rgba = bmp.to_rgba();
            encode_png(&rgba, w, h).map(|png| ("png".to_string(), png))
        };
        let (format, bytes) = match encoded {
            Ok(value) => value,
            Err(_) => {
                error_count += 1;
                continue;
            }
        };
        let bytes = std::sync::Arc::new(bytes);
        out.push(ExtractedImage {
            id: r.id.clone(),
            name: format!("img_{}.{}", r.id, format),
            path: None,
            page: page_number,
            bbox: r.bbox.clone(),
            width: r.pixel_width,
            height: r.pixel_height,
            rotation: r.rotation,
            format: format.clone(),
            duplicate_of: None,
            bytes: std::sync::Arc::clone(&bytes),
        });
        if let Some(raw_bytes) = r.raw_bytes.clone() {
            cache.insert(
                r,
                CachedImage {
                    raw_bytes,
                    id: r.id.clone(),
                    page: page_number,
                    format,
                    bytes,
                },
            );
        }
    }
    RenderedImages {
        images: out,
        error_count,
    }
}

/// Encode RGBA pixel bytes to PNG. Used by both the image-embed path and the
/// `render` module (page rasterization / screenshots).
pub(crate) fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, LiteParseError> {
    encode_png_with_compression(rgba, width, height, crate::config::PngCompression::Fast)
}

pub(crate) fn encode_png_with_compression(
    rgba: &[u8],
    width: u32,
    height: u32,
    compression: crate::config::PngCompression,
) -> Result<Vec<u8>, LiteParseError> {
    let mut png_buf = Vec::new();
    let compression = match compression {
        crate::config::PngCompression::Fast => image::codecs::png::CompressionType::Fast,
        crate::config::PngCompression::Default => image::codecs::png::CompressionType::Default,
        crate::config::PngCompression::Best => image::codecs::png::CompressionType::Best,
    };
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut png_buf,
        compression,
        image::codecs::png::FilterType::Adaptive,
    );
    encoder.write_image(rgba, width, height, image::ColorType::Rgba8.into())?;
    Ok(png_buf)
}

/// Walk image objects on a page and return a stable per-page `ImageRef` for
/// each one. `obj_index` is the index among image-typed page objects (not all
/// page objects), so a later embed pass can pull pixel bytes via
/// `Page::render_image_object`. IDs are scoped to the page number so they
/// remain stable across runs.
struct ExtractedImageRefs {
    refs: Vec<ImageRef>,
    error_count: u32,
}

fn extract_page_image_refs(
    page: &Page,
    page_number: u32,
    include_data: bool,
) -> ExtractedImageRefs {
    let extracted = page.image_objects(IMAGE_MIN_SIZE_PT, IMAGE_MAX_COVERAGE, include_data);
    let refs = extracted
        .images
        .into_iter()
        .enumerate()
        .map(|(i, image)| ImageRef {
            // 1-based image number to match the platform extractor's
            // `img_p%d_%d` naming (C `imageNum` starts at 1).
            id: format!("p{}_{}", page_number, i + 1),
            bbox: Rect {
                x: image.bounds.x,
                y: image.bounds.y,
                width: image.bounds.width,
                height: image.bounds.height,
            },
            obj_index: image.object_index,
            format: if image.jpeg_bytes.is_some() {
                "jpg".to_string()
            } else {
                "png".to_string()
            },
            pixel_width: image.pixel_width,
            pixel_height: image.pixel_height,
            rotation: image.rotation,
            jpeg_bytes: image.jpeg_bytes,
            raw_bytes: image.raw_bytes,
            bits_per_pixel: image.bits_per_pixel,
            colorspace: image.colorspace,
        })
        .collect();
    ExtractedImageRefs {
        refs,
        error_count: extracted.error_count,
    }
}

/// Extract simplified vector graphics from a page. We keep only what the
/// markdown layout pass cares about:
///   - filled paths → a single bounding `Rect` (covers cell backgrounds /
///     code-block fills / banner fills regardless of internal complexity);
///   - stroked paths → one `Stroke` per `LineTo` between consecutive points,
///     plus the implicit closing stroke when a subpath has its close flag set.
///
/// BezierTo segments don't emit strokes (we just advance the current point so
/// later LineTos start from the right place).
fn extract_layout_graphics(paths: &[PathObject]) -> Vec<GraphicPrimitive> {
    let mut out = Vec::new();

    for path in paths {
        // Filled paths: emit one Rect for the full bbox. Cheap signal for
        // cell backgrounds / figure clusters / code-block fills.
        if path.is_filled {
            out.push(GraphicPrimitive::Rect {
                bbox: rectf_to_rect(&path.bbox),
                fill: path.fill_color.as_ref().map(color_to_argb_hex),
                stroke: path.stroke_color.as_ref().map(color_to_argb_hex),
            });
        }

        if !path.is_stroked {
            continue;
        }

        // Stroked paths: walk segments and emit one Stroke per LineTo.
        let color = path.stroke_color.as_ref().map(color_to_argb_hex);
        let mut current: Option<(f32, f32)> = None;
        let mut subpath_start: Option<(f32, f32)> = None;

        for seg in &path.segments {
            match seg.kind {
                SegmentKind::MoveTo => {
                    current = Some((seg.x, seg.y));
                    subpath_start = Some((seg.x, seg.y));
                }
                SegmentKind::LineTo => {
                    if let Some((px, py)) = current {
                        out.push(GraphicPrimitive::Stroke {
                            x1: px,
                            y1: py,
                            x2: seg.x,
                            y2: seg.y,
                            color: color.clone(),
                            width: path.stroke_width,
                        });
                    }
                    current = Some((seg.x, seg.y));
                    if seg.close
                        && let (Some((cx, cy)), Some((sx, sy))) = (current, subpath_start)
                        && (cx - sx).hypot(cy - sy) > 0.01
                    {
                        out.push(GraphicPrimitive::Stroke {
                            x1: cx,
                            y1: cy,
                            x2: sx,
                            y2: sy,
                            color: color.clone(),
                            width: path.stroke_width,
                        });
                    }
                }
                SegmentKind::BezierTo => {
                    // Don't synthesize a stroke for a curve; just advance.
                    current = Some((seg.x, seg.y));
                }
            }
        }
    }

    out
}

const PATH_EPSILON: f32 = 0.001;
const LINE_AXIS_TOLERANCE: f32 = 1.0;

#[derive(Clone)]
struct LineCandidate {
    line: VectorLine,
    shape_index: usize,
}

fn build_vector_graphics(paths: &[PathObject], content_bounds: Option<&Rect>) -> VectorGraphics {
    let painted: Vec<(usize, &PathObject)> = paths
        .iter()
        .enumerate()
        .filter(|(_, path)| path.is_stroked || path.is_filled)
        .collect();
    let white_fill = compute_white_fill_flags(&painted, content_bounds);
    let raw_shapes: Vec<VectorShape> = painted
        .iter()
        .map(|(_, path)| VectorShape {
            bbox: rectf_to_rect(&path.bbox),
            stroke: path.is_stroked,
            stroke_color: path
                .is_stroked
                .then(|| path.stroke_color.as_ref().map(color_to_argb_hex))
                .flatten(),
            fill: path.is_filled,
            fill_color: path
                .is_filled
                .then(|| path.fill_color.as_ref().map(color_to_argb_hex))
                .flatten(),
            has_curve: path
                .segments
                .iter()
                .any(|s| s.kind == SegmentKind::BezierTo),
        })
        .collect();

    // LlamaParse collapses consecutively drawn, same-color solid fills when
    // one contains the other. Preserve order: an intervening paint operation
    // stops merging.
    let mut keep = vec![true; raw_shapes.len()];
    for i in 0..raw_shapes.len().saturating_sub(1) {
        if !keep[i] || !mergeable_shape(&raw_shapes[i]) {
            continue;
        }
        for j in i + 1..raw_shapes.len() {
            if !keep[j] {
                continue;
            }
            if !mergeable_shape(&raw_shapes[j])
                || raw_shapes[i].fill_color != raw_shapes[j].fill_color
            {
                break;
            }
            if rect_contains(&raw_shapes[i].bbox, &raw_shapes[j].bbox) {
                keep[j] = false;
            } else if rect_contains(&raw_shapes[j].bbox, &raw_shapes[i].bbox) {
                keep[i] = false;
            } else {
                break;
            }
        }
    }
    let shapes = raw_shapes
        .iter()
        .cloned()
        .zip(keep.iter().copied())
        .filter_map(|(shape, retained)| retained.then_some(shape))
        .collect();

    let mut horizontal = Vec::new();
    let mut vertical = Vec::new();
    for (shape_index, (_, path)) in painted.iter().enumerate() {
        if white_fill[shape_index] {
            continue;
        }
        let mut current = None;
        for segment in &path.segments {
            match segment.kind {
                SegmentKind::MoveTo => current = Some((segment.x, segment.y)),
                SegmentKind::BezierTo => current = Some((segment.x, segment.y)),
                SegmentKind::LineTo => {
                    if let Some(from) = current {
                        push_axis_line(
                            path,
                            shape_index,
                            from,
                            (segment.x, segment.y),
                            &mut horizontal,
                            &mut vertical,
                        );
                    }
                    current = Some((segment.x, segment.y));
                }
            }
        }
    }
    horizontal.sort_by(|a: &LineCandidate, b| {
        a.line
            .y1
            .total_cmp(&b.line.y1)
            .then(a.line.x1.total_cmp(&b.line.x1))
    });
    vertical.sort_by(|a: &LineCandidate, b| {
        a.line
            .x1
            .total_cmp(&b.line.x1)
            .then(a.line.y1.total_cmp(&b.line.y1))
    });
    let mut lines = merge_axis_lines(horizontal, true, &raw_shapes, &keep);
    lines.extend(merge_axis_lines(vertical, false, &raw_shapes, &keep));
    VectorGraphics { shapes, lines }
}

/// Port of the LlamaParse extract binary's `PATH_FLAGS_WHITE_FILL` heuristic.
/// An unstroked solid-white fill is background paint when it is aligned to
/// the page's content margin, or when it is drawn immediately after and
/// overlapping another white-filled area (pdf writers often paint the
/// background left-to-right, top-to-bottom as a run of adjacent fills).
/// Flagged shapes keep their bbox in `shapes` (still useful for chart /
/// spreadsheet layout detection) but contribute no line segments, which
/// would otherwise create false positives in outlined-table detection.
fn compute_white_fill_flags(
    painted: &[(usize, &PathObject)],
    content_bounds: Option<&Rect>,
) -> Vec<bool> {
    const LINE_DELTA_THRESHOLD: f32 = 1.0;
    let mut flags = vec![false; painted.len()];
    let Some(content) = content_bounds else {
        return flags;
    };
    for i in 0..painted.len() {
        let (path_index, path) = painted[i];
        if path.is_stroked || !path.is_filled {
            continue;
        }
        let Some(fill) = path.fill_color.as_ref() else {
            continue;
        };
        if fill.r != 0xff || fill.g != 0xff || fill.b != 0xff {
            continue;
        }
        let bounds = &path.bbox;
        let margin_aligned = (bounds.left - content.x).abs() < LINE_DELTA_THRESHOLD
            || (bounds.top - content.y).abs() < LINE_DELTA_THRESHOLD
            || (bounds.right - (content.x + content.width)).abs() < LINE_DELTA_THRESHOLD
            || (bounds.bottom - (content.y + content.height)).abs() < LINE_DELTA_THRESHOLD;
        if margin_aligned {
            flags[i] = true;
        } else if i > 0 {
            // Consecutively drawn (adjacent path-object indices; the C
            // implementation additionally requires the same parent object,
            // which the flattened path list approximates) and overlapping a
            // white area extends the blank space.
            let (prev_index, prev) = painted[i - 1];
            if flags[i - 1] && path_index == prev_index + 1 && rects_overlap(&prev.bbox, bounds) {
                flags[i] = true;
            }
        }
    }
    flags
}

/// Bbox overlap with the extract binary's `PDF_POINT_EQUAL_THRESHOLD`
/// slack, in y-down viewport coords (`top <= bottom`).
fn rects_overlap(a: &pdfium::RectF, b: &pdfium::RectF) -> bool {
    !(a.left - PATH_EPSILON > b.right
        || a.right + PATH_EPSILON < b.left
        || a.top - PATH_EPSILON > b.bottom
        || a.bottom + PATH_EPSILON < b.top)
}

fn mergeable_shape(s: &VectorShape) -> bool {
    s.fill && !s.stroke && s.fill_color.is_some()
}
fn rect_contains(a: &Rect, b: &Rect) -> bool {
    a.x - PATH_EPSILON <= b.x + PATH_EPSILON
        && a.y - PATH_EPSILON <= b.y + PATH_EPSILON
        && a.x + a.width + PATH_EPSILON >= b.x + b.width - PATH_EPSILON
        && a.y + a.height + PATH_EPSILON >= b.y + b.height - PATH_EPSILON
}

fn push_axis_line(
    path: &PathObject,
    shape_index: usize,
    a: (f32, f32),
    b: (f32, f32),
    h: &mut Vec<LineCandidate>,
    v: &mut Vec<LineCandidate>,
) {
    let (mut x1, mut y1, mut x2, mut y2) = (a.0, a.1, b.0, b.1);
    let target = if (y2 - y1).abs() < LINE_AXIS_TOLERANCE {
        if x2 < x1 {
            std::mem::swap(&mut x1, &mut x2);
        }
        &mut *h
    } else if (x2 - x1).abs() < LINE_AXIS_TOLERANCE {
        if y2 < y1 {
            std::mem::swap(&mut y1, &mut y2);
        }
        &mut *v
    } else {
        return;
    };
    target.push(LineCandidate {
        shape_index,
        line: VectorLine {
            x1,
            y1,
            x2,
            y2,
            stroke: path.is_stroked,
            stroke_width: path.is_stroked.then_some(path.stroke_width),
            stroke_color: path
                .is_stroked
                .then(|| path.stroke_color.as_ref().map(color_to_argb_hex))
                .flatten(),
            fill: path.is_filled,
            fill_color: path
                .is_filled
                .then(|| path.fill_color.as_ref().map(color_to_argb_hex))
                .flatten(),
        },
    });
}

fn merge_axis_lines(
    mut candidates: Vec<LineCandidate>,
    horizontal: bool,
    shapes: &[VectorShape],
    shape_retained: &[bool],
) -> Vec<VectorLine> {
    let mut retained = vec![true; candidates.len()];
    for i in 0..candidates.len() {
        if !retained[i]
            || is_merged_shape_boundary(&candidates[i], horizontal, shapes, shape_retained)
        {
            retained[i] = false;
            continue;
        }
        for j in i + 1..candidates.len() {
            let same_axis = if horizontal {
                (candidates[i].line.y1 - candidates[j].line.y1).abs() < PATH_EPSILON
            } else {
                (candidates[i].line.x1 - candidates[j].line.x1).abs() < PATH_EPSILON
            };
            if !same_axis {
                break;
            }
            if !retained[j]
                || is_merged_shape_boundary(&candidates[j], horizontal, shapes, shape_retained)
            {
                retained[j] = false;
                continue;
            }
            let a = &candidates[i].line;
            let b = &candidates[j].line;
            let compatible = a.stroke == b.stroke
                && (a.stroke || (a.fill && b.fill))
                && (!a.stroke
                    || ((a.stroke_width.unwrap_or(0.0) - b.stroke_width.unwrap_or(0.0)).abs()
                        < PATH_EPSILON
                        && (a.stroke_color.is_none()
                            || b.stroke_color.is_none()
                            || a.stroke_color == b.stroke_color)))
                && (a.stroke || a.fill_color == b.fill_color);
            if !compatible {
                continue;
            }
            let threshold = PATH_EPSILON
                + if a.stroke {
                    a.stroke_width.unwrap_or(0.0)
                } else {
                    0.0
                };
            let touching = if horizontal {
                b.x1 < a.x2 + threshold && b.x2 > a.x1 - threshold
            } else {
                b.y1 < a.y2 + threshold && b.y2 > a.y1 - threshold
            };
            if touching {
                let b = candidates[j].line.clone();
                if horizontal {
                    candidates[i].line.x1 = candidates[i].line.x1.min(b.x1);
                    candidates[i].line.x2 = candidates[i].line.x2.max(b.x2);
                } else {
                    candidates[i].line.y1 = candidates[i].line.y1.min(b.y1);
                    candidates[i].line.y2 = candidates[i].line.y2.max(b.y2);
                }
                retained[j] = false;
            }
        }
    }
    candidates
        .into_iter()
        .zip(retained)
        .filter_map(|(c, keep)| keep.then_some(c.line))
        .collect()
}

fn is_merged_shape_boundary(
    candidate: &LineCandidate,
    horizontal: bool,
    shapes: &[VectorShape],
    shape_retained: &[bool],
) -> bool {
    if shape_retained[candidate.shape_index] {
        return false;
    }
    let shape = &shapes[candidate.shape_index].bbox;
    if horizontal {
        (candidate.line.y1 - shape.y).abs() < PATH_EPSILON
            || (candidate.line.y1 - (shape.y + shape.height)).abs() < PATH_EPSILON
    } else {
        (candidate.line.x1 - shape.x).abs() < PATH_EPSILON
            || (candidate.line.x1 - (shape.x + shape.width)).abs() < PATH_EPSILON
    }
}

fn rectf_to_rect(r: &RectF) -> Rect {
    Rect {
        x: r.left,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
    }
}

/// Fold typographic punctuation to its ASCII equivalent so extracted text
/// matches plain-ASCII transcriptions: curly quotes → `'`/`"`, the dash family
/// (en/em/figure/non-breaking/minus) → `-`. Applied to every decoded character
/// at extraction time so all output formats are consistent.
fn normalize_punct(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => '"',
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => '-',
        _ => c,
    }
}

/// Character-level text extraction.
///
/// Instead of using PDFium's rect API (which splits text at every font attribute
/// change), we iterate through individual characters and group them by spatial
/// proximity. This keeps words like "A-MEM" together even when internal characters
/// have different font sizes (e.g. small-caps), and keeps punctuation attached to
/// adjacent text (e.g. citation commas/semicolons).
///
/// Segments break at:
/// - Line changes (large vertical shift)
/// - Column breaks (large horizontal gap)
/// - Explicit newline characters
fn extract_page_text_items(
    page: &Page,
    text_page: &TextPage,
    view_box: &RectF,
    glyph_resolver: Option<&dyn crate::GlyphResolver>,
    emit_word_boxes: bool,
    extract_text_metadata: bool,
) -> Result<Vec<TextItem>, LiteParseError> {
    let char_count = text_page.char_count();
    if char_count <= 0 {
        return Ok(Vec::new());
    }

    // Hard limit: gaps larger than this always cause a split (column breaks).
    const MAX_INLINE_GAP: f32 = 15.0;

    let debug = std::env::var("LITEPARSE_DEBUG").is_ok();
    let dbg_gaps = std::env::var("LITEPARSE_DEBUG_GAPS").is_ok();
    // Empirical per-font space calibration: for fonts that expose no
    // space-glyph metric, recover the genuine inter-word gap from the spaces
    // PDFium *does* emit for that font (normalized by rendered em height) and
    // feed it through the same threshold rule the metric path uses.
    let mut font_space_cal: std::collections::HashMap<String, Vec<f32>> =
        std::collections::HashMap::new();

    // Pre-scan: check if ALL text on this page is invisible (render mode 3).
    // Some scanned PDFs have an invisible OCR text layer as the only text.
    // In that case we should use the invisible text rather than skipping it.
    let skip_invisible = should_skip_invisible(text_page, char_count);

    if debug {
        eprintln!("[extract-debug] char_count={char_count}, skip_invisible={skip_invisible}");
    }

    let page_rotation = page.rotation();
    let vp_xform = page.viewport_transform(view_box);
    let mut items: Vec<TextItem> = Vec::new();
    let mut seg = SegmentBuilder::new(emit_word_boxes, extract_text_metadata);
    let garbage_fonts = detect_garbage_unicode_fonts(text_page, char_count);
    let mut glyph_decoder = GlyphDecoder::new(
        std::env::var("LITEPARSE_DEBUG_GLYPH").is_ok(),
        garbage_fonts,
        glyph_resolver,
    );

    for i in 0..char_count {
        let ch = text_page.char_at_unchecked(i);
        let unicode = ch.unicode();
        let is_generated = ch.is_generated();

        // Skip invisible text (render mode 3) only when the page also has visible text.
        // If all text is invisible, it's likely an OCR text layer and we should keep it.
        if skip_invisible && ch.text_render_mode() == Some(3) {
            if debug {
                let c_display = char::from_u32(unicode).unwrap_or('?');
                eprintln!(
                    "[extract-debug] i={i} SKIP invisible char='{c_display}' unicode=0x{unicode:04X}"
                );
            }
            continue;
        }

        // Glyph-name recovery: when the font's unicode mapping is missing or
        // untrusted, resolve the charcode's PostScript glyph name instead.
        let decoded: Option<&str> = if is_generated {
            None
        } else {
            glyph_decoder.decode(&ch, unicode)
        };
        // A glyph the decoder recovered (glyph-name / reverse-cmap / outline-hash
        // resolver) carries correct text even though PDFium still reports a
        // /ToUnicode map error for its raw char code. Don't count these toward the
        // item's unmapped-char tally.
        let recovered = decoded.is_some();

        // Skip null / invalid sentinels (unless the glyph name recovered them)
        if decoded.is_none() && (unicode == 0 || unicode == 0xFFFE || unicode == 0xFFFF) {
            if debug {
                eprintln!("[extract-debug] i={i} SKIP sentinel unicode=0x{unicode:04X}");
            }
            continue;
        }

        // Map to a Rust char, with special-case replacements.
        // Some PDF fonts encode ligatures as control characters; expand them.
        // We use the first char for segment decisions, then append trailing chars.
        let (c, ligature_tail): (char, &str) = if let Some(s) = decoded {
            let mut it = s.chars();
            (it.next().unwrap(), it.as_str())
        } else {
            match unicode {
                0x01 => (' ', ""), // SOH → space: buggy subset fonts (e.g. some
                // Calibri/Cambria embeds) encode the space glyph as 0x01. Left as
                // a raw control char it fuses adjacent words ("StatisticsCheatSheet");
                // as a space it drives the normal pending-space word break below.
                0x02 => ('-', ""),   // STX → hyphen (common in some PDF encodings)
                0x1A => ('f', "f"),  // ff ligature
                0x1B => ('f', "t"),  // ft ligature
                0x1C => ('f', "i"),  // fi ligature
                0x1D => ('T', "h"),  // Th ligature
                0x1E => ('f', "fi"), // ffi ligature
                0x1F => ('f', "l"),  // fl ligature
                _ => match char::from_u32(unicode) {
                    Some(ch_mapped) => (ch_mapped, ""),
                    None => {
                        if debug {
                            eprintln!("[extract-debug] i={i} SKIP invalid unicode=0x{unicode:04X}");
                        }
                        continue;
                    }
                },
            }
        };
        let c = normalize_punct(c);

        // Newlines: flush the current segment
        if c == '\n' || c == '\r' {
            seg.flush(&mut items);
            continue;
        }

        // Whitespace: mark that we're in a pending-space state while retaining
        // the source character code and PDFium-generated distinction.
        if c.is_whitespace() {
            seg.mark_pending_space(is_generated, ch.char_code());
            // Keep PDFium-generated gaps as item boundaries so the emitted
            // item can retain the same trailing-space-generated distinction
            // as the source extractor. The visible text remains trimmed.
            if is_generated && extract_text_metadata {
                seg.flush(&mut items);
            }
            continue;
        }

        // Skip non-space generated characters (synthetic glyphs)
        if is_generated {
            if debug {
                eprintln!(
                    "[extract-debug] i={i} SKIP generated char='{c}' unicode=0x{unicode:04X}"
                );
            }
            continue;
        }

        // Get loose bounds in viewport space for the item bounding box
        let Some(loose_box) = ch.loose_char_box() else {
            if debug {
                eprintln!("[extract-debug] i={i} SKIP no loose_char_box char='{c}'");
            }
            continue;
        };
        let vp_loose = vp_xform.transform_bounds(&loose_box);

        // Skip zero-height characters (phantom dots from dot leader decorations)
        if vp_loose.bottom - vp_loose.top < 0.5 {
            if debug {
                eprintln!(
                    "[extract-debug] i={i} SKIP zero-height char='{c}' height={:.2} vp=({:.1},{:.1})-({:.1},{:.1})",
                    vp_loose.bottom - vp_loose.top,
                    vp_loose.left,
                    vp_loose.top,
                    vp_loose.right,
                    vp_loose.bottom
                );
            }
            continue;
        }

        // Also get strict char box for gap calculation (stays in viewport space)
        let Some(strict_box) = ch.char_box() else {
            if debug {
                eprintln!("[extract-debug] i={i} SKIP no char_box char='{c}'");
            }
            continue;
        };
        let strict_rect = RectF {
            left: strict_box.left as f32,
            top: strict_box.top as f32,
            right: strict_box.right as f32,
            bottom: strict_box.bottom as f32,
        };
        let vp_strict = vp_xform.transform_bounds(&strict_rect);

        if seg.has_content {
            // Use viewport-space coordinates for gap/overlap checks
            let y_tolerance: f32 = 2.0;
            let y_overlap = vp_loose.top < seg.vp_bottom + y_tolerance
                && vp_loose.bottom > seg.vp_top - y_tolerance;

            // Gaps are measured along the segment's writing direction, so a
            // right-to-left run reads exactly like a left-to-right one: positive
            // means "further along the line", negative means "doubled back".
            let gap = seg.gap_to(&vp_strict, c);

            // Detect line change using complementary checks:
            // 1. Strict vertical separation: char's strict top is well below last char's strict bottom
            // 2. Line wrap: char goes back against the writing direction AND strict top is below
            //    last char's strict bottom (even slightly), indicating text wrapped to a new line
            //    within the same text object
            // 3. Very large backward jump: if the char jumps back by more than the current
            //    segment width, it's definitely a new line (handles OCR text with tall bounding
            //    boxes that overlap vertically between lines)
            let strict_below = vp_strict.top > seg.last_char_bottom;
            let large_backward_jump = gap < -5.0;
            let seg_width = seg.vp_right - seg.vp_left;
            let very_large_backward_jump = seg_width > 20.0 && gap < -(seg_width * 0.5);
            let line_changed = vp_strict.top > seg.last_char_bottom + y_tolerance
                || (strict_below && large_backward_jump)
                || very_large_backward_jump;

            // Dot leader detection: break at the boundary between dots and non-dots.
            // This prevents items like "Total . . . . 330,100" from merging.
            let dot_leader_break = if seg.pending_space {
                // With a pending space: break at dot/non-dot transitions
                (c == '.' && seg.has_non_dot_content())
                    || (c != '.' && !seg.has_non_dot_content() && seg.char_count >= 3)
            } else {
                // Without a pending space: break when a dot follows non-dot content
                // with a gap larger than typical intra-word spacing (dot leader dots
                // are spaced apart, unlike periods in abbreviations like "U.S.").
                // A loosely-kerned abbreviation/sentence period sits at ~1x the
                // average char width; genuine no-space dot leaders run far wider
                // (2x+). The 2x cutoff avoids shearing the trailing period off
                // abbreviations like "Sci."/"Chem." when the font kerns the
                // period a hair loose, which would drop it entirely downstream.
                c == '.' && seg.has_non_dot_content() && gap > seg.avg_char_width() * 2.0
            };

            if dbg_gaps && y_overlap && !line_changed && gap > 0.0 {
                let fs = if seg.font_size > 0.0 {
                    seg.font_size
                } else {
                    seg.vp_bottom - seg.vp_top
                };
                let split = gap >= MAX_INLINE_GAP
                    || (seg.pending_space && gap > seg.avg_char_width() * 2.2);
                let loose_gap = seg.loose_gap_to(&vp_strict, c);
                let em_vp = (vp_loose.bottom - vp_loose.top).abs();
                let space_w = ch.font_space_width().map(|w| w * em_vp).unwrap_or(-1.0);
                eprintln!(
                    "[gap] {} gap={:.2} loose={:.2} sw={:.2} g/sw={:.2} fs={:.2} g/fs={:.2} avgcw={:.2} g/cw={:.2} ps={} -> after='{:.20}' next='{}'",
                    if split { "SPLIT" } else { "merge" },
                    gap,
                    loose_gap,
                    space_w,
                    if space_w > 0.0 {
                        loose_gap / space_w
                    } else {
                        0.0
                    },
                    fs,
                    if fs > 0.0 { gap / fs } else { 0.0 },
                    seg.avg_char_width(),
                    gap / seg.avg_char_width().max(0.1),
                    seg.pending_space as u8,
                    seg.text,
                    c,
                );
            }
            if !y_overlap || line_changed || gap >= MAX_INLINE_GAP || dot_leader_break {
                seg.flush(&mut items);
                seg.start(c, &vp_loose, &vp_strict, &ch, recovered, page_rotation);
                seg.append_ligature_tail(ligature_tail);
            } else if seg.pending_space {
                let avg_cw = seg.avg_char_width();
                if gap > avg_cw * 2.2 {
                    seg.flush(&mut items);
                    seg.start(c, &vp_loose, &vp_strict, &ch, recovered, page_rotation);
                    seg.append_ligature_tail(ligature_tail);
                } else {
                    // Genuine inline space PDFium emitted: sample its size
                    // (loose gap / em height) per font, alpha-alpha only, to
                    // calibrate the no-space-metric recovery below.
                    if let Some(fk) = seg.font_name.as_ref() {
                        let prev_alnum = seg
                            .text
                            .chars()
                            .last()
                            .is_some_and(|p| p.is_ascii_alphanumeric());
                        if prev_alnum && c.is_ascii_alphanumeric() {
                            let em_vp = (vp_loose.bottom - vp_loose.top).abs();
                            let loose_gap = seg.loose_gap_to(&vp_strict, c);
                            if em_vp > 0.0 && loose_gap > 0.0 {
                                let s = font_space_cal.entry(fk.clone()).or_default();
                                if s.len() < 512 {
                                    s.push(loose_gap / em_vp);
                                }
                            }
                        }
                    }
                    seg.commit_pending_space();
                    seg.push_char(c, &vp_loose, &vp_strict, &ch, recovered);
                    seg.append_ligature_tail(ligature_tail);
                }
            } else {
                // Missing-space recovery: PDFium sometimes omits the space glyph
                // between words, fusing them ("of the" -> "ofthe"). Detect it from
                // the advance-relative gap (measured against the previous char's
                // LOOSE right edge, so intra-word kerning/overhang is subtracted out)
                // compared to the font's actual ASCII-space advance. Only fires
                // between two ASCII alphanumerics, which keeps abbreviation dots,
                // hyphens, and CJK untouched. When the font exposes no space-glyph
                // metric (common in embedded subset fonts) fall back to a fraction
                // of the rendered em height as the space estimate.
                let em_vp = (vp_loose.bottom - vp_loose.top).abs();
                let space_w = ch.font_space_width().map(|w| w * em_vp).unwrap_or(0.0);
                let loose_gap = seg.loose_gap_to(&vp_strict, c);
                let both_alnum = c.is_ascii_alphanumeric()
                    && seg
                        .text
                        .chars()
                        .last()
                        .is_some_and(|p| p.is_ascii_alphanumeric());
                let thresh = if space_w > 0.0 {
                    0.7 * space_w
                } else {
                    // No space-glyph metric. Prefer an empirically-recovered
                    // space width (median genuine-space ratio for this font ×
                    // em height) run through the same 0.7 factor as the metric
                    // path; fall back to a fixed em fraction when we lack
                    // enough samples for the font.
                    let calibrated = seg
                        .font_name
                        .as_ref()
                        .and_then(|fk| font_space_cal.get(fk))
                        .filter(|s| s.len() >= MIN_SPACE_CAL_SAMPLES)
                        .and_then(|s| median_f32(s))
                        .map(|ratio| 0.7 * ratio * em_vp);
                    calibrated.unwrap_or(0.35 * em_vp)
                };
                if both_alnum && thresh > 0.0 && loose_gap > thresh {
                    seg.text.push(' ');
                    seg.break_word();
                }
                seg.push_char(c, &vp_loose, &vp_strict, &ch, recovered);
                seg.append_ligature_tail(ligature_tail);
            }
        } else {
            seg.start(c, &vp_loose, &vp_strict, &ch, recovered, page_rotation);
            seg.append_ligature_tail(ligature_tail);
        }
    }

    seg.flush(&mut items);

    // Drop items entirely outside the page view box. Print-spread / imposed
    // PDFs carry the neighbouring page's text at x beyond the page edge in
    // the same content stream; viewers never show it. Partially-visible
    // items are kept.
    // Item coordinates have already been transformed into the
    // rotation-adjusted viewport. Clip against dimensions in that same space;
    // using the raw CropBox dimensions here drops the right/bottom portion of
    // /Rotate 90 and /Rotate 270 pages.
    let (vb_w, vb_h) = page.viewport_size(view_box);
    let pre_clip_count = items.len();
    items.retain(|it| {
        it.x < vb_w
            && it.x + it.width.max(0.1) > 0.0
            && it.y < vb_h
            && it.y + it.height.max(0.1) > 0.0
    });
    if debug && items.len() < pre_clip_count {
        eprintln!(
            "[extract-debug] off-page clip removed {} items",
            pre_clip_count - items.len()
        );
    }

    if debug {
        eprintln!("[extract-debug] items before dedup: {}", items.len());
    }

    // Dedup: remove items with identical text and overlapping bounding boxes.
    // Some PDFs (especially those with chart/figure annotations) produce duplicate
    // text objects at the same position.
    let pre_dedup_count = items.len();
    dedup_overlapping_items(&mut items, debug);

    if debug && items.len() < pre_dedup_count {
        eprintln!(
            "[extract-debug] dedup removed {} items ({} → {})",
            pre_dedup_count - items.len(),
            pre_dedup_count,
            items.len()
        );
    }

    Ok(items)
}

/// Remove duplicate text items: exact text matches with any bbox overlap,
/// and near-duplicates (different text) with high bbox overlap (>50% area).
fn dedup_overlapping_items(items: &mut Vec<TextItem>, debug: bool) {
    if items.len() < 2 {
        return;
    }

    let mut keep = vec![true; items.len()];
    for i in 0..items.len() {
        if !keep[i] {
            continue;
        }
        for j in (i + 1)..items.len() {
            if !keep[j] {
                continue;
            }

            let a = &items[i];
            let b = &items[j];

            // Diagonal (non-right-angle) text has a *loose* axis-aligned
            // bounding box — the hull of a rotated glyph run is far larger than
            // the ink, so two stacked lines of the same skewed block (e.g.
            // "Paris has the" above "eiffel tower" at 51°) report heavy bbox
            // overlap even though the glyphs never touch. Dedup keys off AABB
            // overlap, so it would wrongly drop one of those lines. Skip the
            // comparison entirely when either item is diagonal; true duplicate
            // stamps are upright and still handled below.
            if is_diagonal_rotation(a.rotation) || is_diagonal_rotation(b.rotation) {
                continue;
            }

            // Compute intersection area
            let ix_left = a.x.max(b.x);
            let ix_right = (a.x + a.width).min(b.x + b.width);
            let iy_top = a.y.max(b.y);
            let iy_bottom = (a.y + a.height).min(b.y + b.height);

            if ix_left >= ix_right || iy_top >= iy_bottom {
                continue; // no overlap
            }

            let intersection = (ix_right - ix_left) * (iy_bottom - iy_top);
            let area_a = a.width * a.height;
            let area_b = b.width * b.height;
            let smaller_area = area_a.min(area_b);

            if items[i].text == items[j].text {
                // Exact text match: require strong bounding-box overlap before
                // dedup. The same word routinely appears more than once on a
                // page in different positions; firing on any overlap would drop
                // a legitimate occurrence when two identical words' bboxes share
                // even a sliver of area (e.g. one column's word vertically
                // adjacent to another column's identical word with a slack
                // loose-box), corrupting that line.
                //
                // Require ≥50% overlap of the smaller item — same threshold
                // as the non-exact branch. True duplicate stamps overlap
                // essentially 100%; unrelated repeats overlap 0%.
                let strong_overlap = smaller_area > 0.0 && intersection / smaller_area > 0.5;
                if !strong_overlap {
                    continue;
                }
                if debug {
                    eprintln!(
                        "[extract-debug] DEDUP exact-match drop i={i} text='{}' at ({:.1},{:.1} {}x{}) in favor of j={j} at ({:.1},{:.1} {}x{}) overlap_ratio={:.2}",
                        items[i].text,
                        items[i].x,
                        items[i].y,
                        items[i].width,
                        items[i].height,
                        items[j].x,
                        items[j].y,
                        items[j].width,
                        items[j].height,
                        intersection / smaller_area
                    );
                }
                keep[i] = false;
                break; // i is gone, move to next i
            } else if smaller_area > 0.0 && intersection / smaller_area > 0.5 {
                // Different text but >50% overlap of the smaller item:
                // likely overlapping text layers (e.g. old/new branding).
                // Keep the later one (rendered on top in PDF paint order).
                //
                // However, skip dedup when the items have very different sizes
                // (area ratio > 5x). This happens when a small cell value sits
                // inside a row-spanning element like a dotted leader — these are
                // separate content, not overlapping layers.
                let larger_area = area_a.max(area_b);
                if larger_area / smaller_area > 5.0 {
                    if debug {
                        eprintln!(
                            "[extract-debug] DEDUP skip (area ratio {:.1}x) i={i} text='{}' j={j} text='{}'",
                            larger_area / smaller_area,
                            items[i].text,
                            items[j].text
                        );
                    }
                    continue;
                }
                if debug {
                    eprintln!(
                        "[extract-debug] DEDUP overlap drop i={i} text='{}' at ({:.1},{:.1} {}x{}) in favor of j={j} text='{}' at ({:.1},{:.1} {}x{}) overlap_ratio={:.2}",
                        items[i].text,
                        items[i].x,
                        items[i].y,
                        items[i].width,
                        items[i].height,
                        items[j].text,
                        items[j].x,
                        items[j].y,
                        items[j].width,
                        items[j].height,
                        intersection / smaller_area
                    );
                }
                keep[i] = false;
                break; // i is gone, move to next i
            }
        }
    }

    let mut idx = 0;
    items.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

/// True when `rotation` (degrees) is more than 2° off the nearest right angle
/// (0/90/180/270). A page's diagonal watermark/stamp text
/// is classified identically on both sides.
fn is_diagonal_rotation(rotation: f32) -> bool {
    let nearest_right_angle = (rotation / 90.0).round() * 90.0;
    (rotation - nearest_right_angle).abs() > 2.0
}

/// Apply caller-requested content filters to already-extracted (and
/// OCR-merged) pages, in place, just before grid projection:
///
/// * `skip_diagonal` drops skewed text (watermarks, rotated stamps).
/// * `crop_box` keeps only items lying *entirely* inside the surviving page
///   region — fractions cropped from each side, top-left origin.
///
/// Running here (after OCR merge, before projection) means both native and
/// OCR-sourced items are filtered and removed text never reaches the output.
/// No-op when neither filter is requested.
pub(crate) fn apply_content_filters(
    pages: &mut [LitePage],
    crop_box: Option<&crate::config::CropBox>,
    skip_diagonal: bool,
) {
    if crop_box.is_none() && !skip_diagonal {
        return;
    }
    for page in pages.iter_mut() {
        if skip_diagonal {
            page.text_items
                .retain(|it| !is_diagonal_rotation(it.rotation));
        }
        if let Some(cb) = crop_box {
            let w = page.page_width;
            let h = page.page_height;
            let min_x = cb.left * w;
            let max_x = (1.0 - cb.right) * w;
            let min_y = cb.top * h;
            let max_y = (1.0 - cb.bottom) * h;
            page.text_items.retain(|it| {
                it.x >= min_x
                    && it.x + it.width <= max_x
                    && it.y >= min_y
                    && it.y + it.height <= max_y
            });
        }
    }
}

/// Adjust character angle for page rotation.
/// PDFium returns counter-clockwise angle in PDF space; page /Rotate is clockwise.
fn adjust_angle_for_rotation(angle_rad: f32, page_rotation: i32) -> f32 {
    use std::f32::consts::PI;
    let mut a = angle_rad;
    match page_rotation {
        1 => a -= 3.0 * PI / 2.0, // 90°
        2 => a -= PI,             // 180°
        3 => a -= PI / 2.0,       // 270°
        _ => {}
    }
    a = a.rem_euclid(2.0 * PI);
    a
}

/// Decompose scale factors from a 2D affine matrix.
/// Computes eigenvalues of M^T * M.
fn decompose_scale(m: &pdfium::Matrix) -> (f32, f32) {
    let (a, b, c, d) = (m.a as f64, m.b as f64, m.c as f64, m.d as f64);
    // M^T * M
    let mt_a = a * a + b * b;
    let mt_b = a * c + b * d;
    let mt_d = c * c + d * d;
    let first = (mt_a + mt_d) / 2.0;
    let disc = ((mt_a + mt_d).powi(2) - 4.0 * (mt_a * mt_d - mt_b * mt_b)).sqrt() / 2.0;
    let sx = (first + disc).sqrt();
    let sy = (first - disc).sqrt();
    let sx = if sx.is_nan() { 1.0 } else { sx };
    let sy = if sy.is_nan() { 1.0 } else { sy };
    (sx as f32, sy as f32)
}

/// Minimum genuine-space samples required before trusting per-font calibration.
const MIN_SPACE_CAL_SAMPLES: usize = 6;

/// Median of a slice of finite, non-negative f32 values. Returns None if empty.
fn median_f32(values: &[f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut v: Vec<f32> = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        Some((v[mid - 1] + v[mid]) / 2.0)
    } else {
        Some(v[mid])
    }
}

/// Check if a font is "buggy" based on its name and type.
fn is_buggy_font(font_name: &str, font_type: FontType) -> bool {
    // TrueType subset fonts: name starts with "TT" or contains "+TT"
    if font_name.starts_with("TT") || font_name.contains("+TT") {
        return true;
    }
    // Type1 fonts with 6-char prefix + underscore: "ABCDEF_..."
    if font_type == FontType::Type1 && font_name.len() >= 7 {
        let bytes = font_name.as_bytes();
        if bytes[6] == b'_' {
            return true;
        }
    }
    false
}

/// Check if a Unicode codepoint indicates buggy encoding.
/// C0 controls (<=0x1F), DEL + C1 controls (0x7F-0x9F), and the private use area.
/// None of these are ever legitimate rendered text; C1 controls in particular
/// are emitted by a common class of subset fonts that mangle ToUnicode into
/// the 0x80-0x9F range.
fn is_buggy_codepoint(unicode: u32) -> bool {
    unicode <= 0x1F || (0x7F..=0x9F).contains(&unicode) || (unicode > 0xE000 && unicode <= 0xF8FF)
}

fn color_to_argb_hex(c: &pdfium::Color) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", c.a, c.r, c.g, c.b)
}

/// Per-page glyph-name-based unicode recovery (fork API).
///
/// When a font has no /ToUnicode CMap, PDFium derives unicode from the
/// encoding alone — garbage for custom/Identity encodings (Mode 10 glyph
/// soup), and guessed control-code expansions for ligatures (Mode 16). The
/// PostScript glyph name the font assigns to the charcode (from /Encoding
/// /Differences or the embedded font program) resolved against the Adobe
/// Glyph List is the authoritative signal in both cases.
struct GlyphDecoder<'a> {
    fonts: std::collections::HashMap<usize, FontGlyphInfo>,
    /// Chars arrive in runs per text object; cache the last object's font key
    /// to skip the FPDFTextObj_GetFont FFI call on the common path.
    last_obj: usize,
    last_key: usize,
    /// Font handles whose /ToUnicode the prescan flagged as garbage (high
    /// fraction of control/PUA unicodes across the page).
    garbage_fonts: std::collections::HashSet<usize>,
    /// Optional last-resort recovery hook for untrusted glyphs that the
    /// built-in glyph-name / reverse-cmap recovery could not decode.
    resolver: Option<&'a dyn crate::GlyphResolver>,
    debug: bool,
}

struct FontGlyphInfo {
    font: Font,
    /// No /ToUnicode and no standard base encoding (or the prescan flagged
    /// the ToUnicode as garbage): PDFium's unicode values for this font are
    /// untrusted, so every charcode gets a recovery try.
    untrusted: bool,
    /// The font's name matches the buggy-subset heuristic while
    /// declaring a *standard* base encoding (e.g. MacRomanEncoding) — the
    /// encoding is a lie, so PDFium derives glyph names from it that are just
    /// as wrong as the unicode. Skip glyph-name recovery for these and rely on
    /// the embedded cmap / outline-hash resolver instead (matches the C path,
    /// which ignores glyph names for `PARSE_TEXT_FONT_BUGGY` fonts).
    encoding_lies: bool,
    /// charcode → resolved replacement text (None = unrecoverable)
    cache: std::collections::HashMap<u32, Option<String>>,
    /// Lazily-built glyph_index → unicode map from the embedded font
    /// program's cmap table (None = not yet built, Some(None) = unavailable).
    reverse_cmap: Option<Option<std::collections::HashMap<u32, u32>>>,
}

impl<'a> GlyphDecoder<'a> {
    fn new(
        debug: bool,
        garbage_fonts: std::collections::HashSet<usize>,
        resolver: Option<&'a dyn crate::GlyphResolver>,
    ) -> Self {
        Self {
            fonts: std::collections::HashMap::new(),
            garbage_fonts,
            resolver,
            last_obj: 0,
            last_key: 0,
            debug,
        }
    }

    /// Returns replacement text for this char when its glyph name resolves
    /// and the current unicode is suspicious (control/PUA/sentinel/map-error)
    /// or the font's unicode mapping is untrusted altogether.
    fn decode(&mut self, ch: &pdfium::TextChar, unicode: u32) -> Option<&str> {
        let cheap_suspicious = matches!(unicode, 0 | 0xFFFE | 0xFFFF)
            || (unicode < 0x20 && !matches!(unicode, 0x09 | 0x0A | 0x0D))
            || (0xE000..=0xF8FF).contains(&unicode);

        let obj_ptr = ch.text_object()?;
        let obj = obj_ptr as usize;
        let key = if obj == self.last_obj {
            self.last_key
        } else {
            let font = unsafe { Font::from_text_object(obj_ptr) }?;
            let key = font.handle() as usize;
            let debug = self.debug;
            let garbage = self.garbage_fonts.contains(&key);
            self.fonts.entry(key).or_insert_with(|| {
                let has_to_unicode = font.has_to_unicode();
                let encoding = font.encoding();
                // Embedded subset fonts whose name matches the "buggy
                // font" heuristic (TrueType `+TT` / Type1 `......_` subset tags)
                // routinely lie about their encoding: a standard base encoding
                // (e.g. MacRomanEncoding) decodes to a shifted alphabet because
                // the embedded glyph program doesn't follow it. PDFium's unicode
                // for these looks plausible (printable letters), so the cheap
                // per-glyph suspicion checks never fire — flag the whole font
                // untrusted so every glyph goes through recovery. Mirrors the C
                // path's `PARSE_TEXT_FONT_BUGGY` name flagging (embedded &&
                // isBuggyFont).
                let name_buggy = font.is_embedded()
                    && font
                        .base_name()
                        .is_some_and(|name| is_buggy_font(&name, font.font_type()));
                let untrusted = garbage
                    || name_buggy
                    || (!has_to_unicode
                        && !matches!(
                            encoding.as_deref(),
                            Some("WinAnsiEncoding")
                                | Some("MacRomanEncoding")
                                | Some("MacExpertEncoding")
                                | Some("StandardEncoding")
                        ));
                if debug {
                    eprintln!(
                        "[glyph] font={:?} to_unicode={} encoding={:?} garbage={} name_buggy={} untrusted={}",
                        font.base_name(),
                        has_to_unicode,
                        encoding,
                        garbage,
                        name_buggy,
                        untrusted
                    );
                }
                FontGlyphInfo {
                    font,
                    untrusted,
                    encoding_lies: name_buggy,
                    cache: std::collections::HashMap::new(),
                    reverse_cmap: None,
                }
            });
            self.last_obj = obj;
            self.last_key = key;
            key
        };
        let info = self.fonts.get_mut(&key)?;

        // map-error FFI check is the expensive part of "suspicious"; only
        // consult it when the cheap checks and font trust don't decide.
        if !info.untrusted && !cheap_suspicious && !ch.has_unicode_map_error() {
            return None;
        }
        let debug = self.debug;
        let resolver = self.resolver;

        let char_code = ch.char_code();
        let encoding_lies = info.encoding_lies;
        let FontGlyphInfo {
            font,
            cache,
            reverse_cmap,
            ..
        } = info;
        let resolved = cache
            .entry(char_code)
            .or_insert_with(|| {
                let name = font.char_glyph_name(char_code);
                // Glyph names of buggy-subset fonts are derived from a lying
                // base encoding, so they mis-decode exactly like PDFium's
                // unicode (e.g. charcode 0x53 → name "S" but the glyph draws
                // 'R'). Skip name recovery for them so the embedded-cmap /
                // outline-hash resolver below — the only trustworthy signals —
                // get the chance to correct the glyph.
                let resolved = if encoding_lies {
                    None
                } else {
                    name.as_deref()
                        .and_then(resolve_glyph_name)
                        .filter(|r| r.chars().all(|c| !c.is_control()))
                };
                // Fallback: reverse-map the glyph index through the embedded
                // font program's own cmap table.
                let resolved = resolved.or_else(|| {
                    let glyph = font.char_glyph_index(char_code)?;
                    let map = reverse_cmap
                        .get_or_insert_with(|| {
                            let data = font.font_data();
                            let map = data.as_deref().and_then(crate::font_cmap::reverse_cmap);
                            if debug {
                                eprintln!(
                                    "[glyph] reverse_cmap build: data={:?} bytes, entries={:?}",
                                    data.as_ref().map(|d| d.len()),
                                    map.as_ref().map(|m| m.len())
                                );
                            }
                            map
                        })
                        .as_ref()?;
                    let u = *map.get(&glyph)?;
                    if (0xE000..=0xF8FF).contains(&u) {
                        return None;
                    }
                    // Synthetic subset cmaps just echo the charcode back
                    // (charcode-identity, not semantic unicode). A recovery
                    // that "resolves" to the charcode itself is that
                    // signature, not a real mapping — keep PDFium's value.
                    if u == char_code && u != unicode {
                        return None;
                    }
                    let c = char::from_u32(u).filter(|c| !c.is_control())?;
                    Some(match crate::glyph_names::presentation_form_expansion(c) {
                        Some(s) => s.to_string(),
                        None => c.to_string(),
                    })
                });
                // Last resort: hand the glyph's vector outline to the injected
                // resolver. Only reached for untrusted glyphs the deterministic 
                // recovery above could not decode.
                let resolved = resolved.or_else(|| {
                    let resolver = resolver?;
                    let segments =
                        font.glyph_path_segments(char_code, crate::GLYPH_RESOLVER_FONT_SIZE)?;
                    let text = resolver.resolve(&segments)?;
                    if text.is_empty() || text.chars().any(|c| c.is_control()) {
                        return None;
                    }
                    if debug {
                        eprintln!("[glyph] cc=0x{char_code:04X} resolver -> {text:?}");
                    }
                    Some(text)
                });
                if debug {
                    eprintln!(
                        "[glyph] cc=0x{char_code:04X} unicode=0x{unicode:04X} name={name:?} -> {resolved:?}"
                    );
                }
                resolved
            });
        // Don't double-expand a ligature PDFium already split. With no
        // /ToUnicode, PDFium derives per-char unicodes from the glyph names
        // itself, expanding a single ligature glyph (e.g. the "fi" glyph at
        // char_code 0x02) into separate 'f' and 'i' TextChar entries that all
        // share that one char_code. Resolving the multi-char glyph name ("fi")
        // once per entry would emit "fi"+"fi" → "fifind". When PDFium already
        // gave a clean (non-suspicious) char that is part of the resolved
        // string, it has done the expansion — keep its char. Suspicious-char
        // recoveries (control-code ligatures, glyph soup) still expand.
        if let Some(r) = resolved.as_deref()
            && r.chars().count() > 1
            && !cheap_suspicious
            && let Some(u) = char::from_u32(unicode)
            && r.contains(u)
        {
            return None;
        }
        resolved.as_deref()
    }
}

/// Prescan: flag fonts whose /ToUnicode maps a high fraction of chars into
/// control/PUA/sentinel codepoints — a structurally present but garbage CMap
/// (e.g. `text_simple__spd`). Chars from flagged fonts get glyph-name /
/// reverse-cmap recovery even when their individual unicode looks plausible.
fn detect_garbage_unicode_fonts(
    text_page: &TextPage,
    char_count: i32,
) -> std::collections::HashSet<usize> {
    let mut counts: std::collections::HashMap<usize, (u32, u32)> = std::collections::HashMap::new();
    let mut last_obj: usize = 0;
    let mut last_key: usize = 0;
    for i in 0..char_count {
        let ch = text_page.char_at_unchecked(i);
        if ch.is_generated() {
            continue;
        }
        let unicode = ch.unicode();
        if matches!(unicode, 0x09 | 0x0A | 0x0D | 0x20) {
            continue;
        }
        let Some(obj_ptr) = ch.text_object() else {
            continue;
        };
        let obj = obj_ptr as usize;
        let key = if obj == last_obj {
            last_key
        } else {
            let Some(font) = (unsafe { Font::from_text_object(obj_ptr) }) else {
                continue;
            };
            last_obj = obj;
            last_key = font.handle() as usize;
            last_key
        };
        let entry = counts.entry(key).or_insert((0, 0));
        entry.0 += 1;
        let suspicious = matches!(unicode, 0 | 0xFFFE | 0xFFFF)
            || unicode < 0x20
            || (0xE000..=0xF8FF).contains(&unicode);
        if suspicious {
            entry.1 += 1;
        }
    }
    counts
        .into_iter()
        .filter(|&(_, (total, suspicious))| total >= 20 && suspicious * 10 >= total)
        .map(|(key, _)| key)
        .collect()
}

/// Whether a just-decoded glyph should count toward its item's unmapped-char
/// tally (which sets `TextItem::has_unicode_map_error` by majority vote in
/// `flush`).
fn counts_as_unmapped(recovered: bool, raw_map_error: bool) -> bool {
    !recovered && raw_map_error
}

/// Accumulates characters into a single TextItem segment.
struct SegmentBuilder {
    text: String,
    // Viewport-space bounding box (union of loose bounds, top-left origin)
    vp_left: f32,
    vp_right: f32,
    vp_top: f32,
    vp_bottom: f32,
    // Right edge of last char strict bounds (for gap calculation)
    last_char_right: f32,
    // Right edge of last char LOOSE bounds (advance-relative gap calculation)
    last_char_loose_right: f32,
    // Left edges of the same boxes. Right-to-left runs (Hebrew, Arabic) advance
    // leftward, so their trailing edge is the LEFT one; `gap_to` picks the pair
    // that matches the segment's writing direction.
    last_char_left: f32,
    last_char_loose_left: f32,
    // Writing direction of the segment, latched from its first strong-direction
    // character. Stays `None` through leading neutrals (digits, punctuation) so
    // a segment opening with "(" still picks up RTL from the Hebrew/Arabic
    // letter that follows.
    dir_rtl: Option<bool>,
    // Bottom of last char strict bounds (for line-change detection)
    last_char_bottom: f32,
    // Count of non-space characters (for avg width calculation)
    char_count: usize,
    // Count of characters whose Unicode came from PDFium's char-code fallback
    // (no usable ToUnicode / glyph-name mapping, e.g. Type3 fonts).
    unmapped_char_count: usize,
    // Font metadata (captured from the first character)
    font_name: Option<String>,
    font_size: f32,
    font_height: Option<f32>,
    font_ascent: Option<f32>,
    font_descent: Option<f32>,
    font_weight: Option<i32>,
    font_flags: Option<i32>,
    font_is_buggy: bool,
    font_is_embedded: bool,
    font: Option<Font>,
    rotation_deg: f32,
    text_width: f32,
    mcid: Option<i32>,
    fill_color: Option<String>,
    stroke_color: Option<String>,
    char_codes: Vec<u32>,
    pending_space_char_codes: Vec<u32>,
    pending_space_generated: bool,
    has_content: bool,
    pending_space: bool,
    // Per-word sub-boxes, finalized at each inter-word space break. The
    // currently-open word is accumulated in the `word_*` fields below and
    // flushed into `words` by `break_word`.
    words: Vec<WordBox>,
    cur_word: String,
    word_left: f32,
    word_right: f32,
    word_top: f32,
    word_bottom: f32,
    word_has: bool,
    // When false, per-word tracking is skipped entirely and `words` stays empty.
    emit_words: bool,
    // When false, source-code/trailing-space metadata is not accumulated.
    extract_text_metadata: bool,
}

impl SegmentBuilder {
    fn new(emit_words: bool, extract_text_metadata: bool) -> Self {
        Self {
            text: String::new(),
            vp_left: f32::MAX,
            vp_right: f32::MIN,
            vp_top: f32::MAX,
            vp_bottom: f32::MIN,
            last_char_right: f32::MIN,
            last_char_loose_right: f32::MIN,
            last_char_left: f32::MAX,
            last_char_loose_left: f32::MAX,
            dir_rtl: None,
            last_char_bottom: f32::MIN,
            char_count: 0,
            unmapped_char_count: 0,
            font_name: None,
            font_size: 0.0,
            font_height: None,
            font_ascent: None,
            font_descent: None,
            font_weight: None,
            font_flags: None,
            font_is_buggy: false,
            font_is_embedded: false,
            font: None,
            rotation_deg: 0.0,
            text_width: 0.0,
            mcid: None,
            fill_color: None,
            stroke_color: None,
            char_codes: Vec::new(),
            pending_space_char_codes: Vec::new(),
            pending_space_generated: false,
            has_content: false,
            pending_space: false,
            words: Vec::new(),
            cur_word: String::new(),
            word_left: f32::MAX,
            word_right: f32::MIN,
            word_top: f32::MAX,
            word_bottom: f32::MIN,
            word_has: false,
            emit_words,
            extract_text_metadata,
        }
    }

    /// Extend the currently-open word with a character's loose box, opening a
    /// fresh word if none is active. No-op unless word emission is enabled.
    fn add_word_char(&mut self, c: char, vp_loose: &RectF) {
        if !self.emit_words {
            return;
        }
        if self.word_has {
            self.word_left = self.word_left.min(vp_loose.left);
            self.word_right = self.word_right.max(vp_loose.right);
            self.word_top = self.word_top.min(vp_loose.top);
            self.word_bottom = self.word_bottom.max(vp_loose.bottom);
        } else {
            self.cur_word.clear();
            self.word_left = vp_loose.left;
            self.word_right = vp_loose.right;
            self.word_top = vp_loose.top;
            self.word_bottom = vp_loose.bottom;
            self.word_has = true;
        }
        self.cur_word.push(c);
    }

    /// Finalize the open word (if any) into `words` and reset the accumulator.
    /// Called at each inter-word space and at segment flush.
    fn break_word(&mut self) {
        if !self.word_has {
            return;
        }
        let trimmed = self.cur_word.trim();
        if !trimmed.is_empty() {
            self.words.push(WordBox {
                text: trimmed.to_string(),
                x: self.word_left,
                y: self.word_top,
                width: self.word_right - self.word_left,
                height: self.word_bottom - self.word_top,
            });
        }
        self.cur_word.clear();
        self.word_has = false;
    }

    /// Gap from the segment's last character to the incoming one, measured
    /// along the writing direction against the previous char's *trailing* edge
    /// (its right edge for left-to-right text, its left edge for right-to-left).
    /// Positive means the incoming char sits further along the line, negative
    /// means it doubled back — the same sign convention in both directions, so
    /// every threshold downstream stays direction-agnostic.
    fn gap_to(&self, vp_strict: &RectF, c: char) -> f32 {
        if self.dir_is_rtl(c) {
            self.last_char_left - vp_strict.right
        } else {
            vp_strict.left - self.last_char_right
        }
    }

    /// As [`gap_to`](Self::gap_to), but against the previous char's LOOSE box,
    /// which subtracts out intra-word kerning/overhang. This is the
    /// advance-relative gap the missing-space recovery compares to the font's
    /// space width.
    fn loose_gap_to(&self, vp_strict: &RectF, c: char) -> f32 {
        if self.dir_is_rtl(c) {
            self.last_char_loose_left - vp_strict.right
        } else {
            vp_strict.left - self.last_char_loose_right
        }
    }

    /// Writing direction to use for the pair (segment tail, `c`). Falls back to
    /// the incoming character while the segment has only seen neutrals.
    fn dir_is_rtl(&self, c: char) -> bool {
        self.dir_rtl.unwrap_or_else(|| is_rtl_char(c))
    }

    /// Record `c`'s contribution to the segment's writing direction. Neutral
    /// characters (digits, punctuation, spaces) leave it untouched.
    fn note_direction(&mut self, c: char) {
        if is_rtl_char(c) {
            self.dir_rtl = Some(true);
        } else if c.is_alphabetic() {
            self.dir_rtl = Some(false);
        }
    }

    /// Average width of non-space characters in the current segment.
    /// Prefers actual glyph widths (text_width) over bbox width, since bbox
    /// includes inter-character gaps that inflate the average and cause
    /// separate table cell values to merge into one item.
    fn avg_char_width(&self) -> f32 {
        if self.char_count == 0 {
            return 5.0;
        }
        if self.text_width > 0.0 {
            self.text_width / self.char_count as f32
        } else {
            (self.vp_right - self.vp_left) / self.char_count as f32
        }
    }

    /// Start a new segment with the given character.
    fn start(
        &mut self,
        c: char,
        vp_loose: &RectF,
        vp_strict: &RectF,
        ch: &pdfium::TextChar,
        recovered: bool,
        page_rotation: i32,
    ) {
        self.text.clear();
        self.text.push(c);
        self.vp_left = vp_loose.left;
        self.vp_right = vp_loose.right;
        self.vp_top = vp_loose.top;
        self.vp_bottom = vp_loose.bottom;
        self.last_char_right = vp_strict.right;
        self.last_char_loose_right = vp_loose.right;
        self.last_char_left = vp_strict.left;
        self.last_char_loose_left = vp_loose.left;
        self.last_char_bottom = vp_strict.bottom;
        self.dir_rtl = None;
        self.note_direction(c);
        self.char_count = 1;
        self.unmapped_char_count = if counts_as_unmapped(recovered, ch.has_unicode_map_error()) {
            1
        } else {
            0
        };
        self.has_content = true;
        self.pending_space = false;
        if self.extract_text_metadata {
            self.pending_space_char_codes.clear();
        }
        self.pending_space_generated = false;
        self.words.clear();
        self.word_has = false;
        self.add_word_char(c, vp_loose);
        self.text_width = 0.0;
        if self.extract_text_metadata {
            self.char_codes.clear();
            self.char_codes.push(ch.char_code());
        }
        self.font_is_buggy = false;
        self.font_is_embedded = false;
        self.font = None;

        // Font info
        if let Some((name, flags)) = ch.font_info() {
            self.font_name = Some(name);
            self.font_flags = Some(flags);
        } else {
            self.font_name = None;
            self.font_flags = None;
        }

        let fs = ch.font_size() as f32;
        self.font_size = if fs > 0.0 {
            fs
        } else {
            (vp_loose.bottom - vp_loose.top).abs()
        };

        self.font_weight = {
            let w = ch.font_weight();
            if w > 0 { Some(w) } else { None }
        };

        // Angle adjusted for page rotation
        let angle_rad = ch.angle();
        self.rotation_deg = if angle_rad >= 0.0 {
            adjust_angle_for_rotation(angle_rad, page_rotation).to_degrees()
        } else {
            0.0
        };

        // Font object for ascent/descent/glyph widths/buggy detection
        if let Some(obj) = ch.text_object() {
            if let Some(font) = unsafe { Font::from_text_object(obj) } {
                if let Some(name) = font.base_name() {
                    let ft = font.font_type();
                    self.font_is_embedded = font.is_embedded();

                    if self.font_is_embedded && is_buggy_font(&name, ft) {
                        self.font_is_buggy = true;
                    }

                    self.font_name = Some(name);
                }

                self.font_ascent = font.ascent(self.font_size);
                self.font_descent = font.descent(self.font_size);

                // Glyph width for first char
                let char_code = ch.char_code();
                if let Some(w) = font.glyph_width_from_char_code(char_code, self.font_size) {
                    self.text_width += w;
                }

                self.font = Some(font);
            }

            // fontHeight = fontSize * scaleY
            if let Some(matrix) = ch.matrix() {
                let (_sx, sy) = decompose_scale(&matrix);
                self.font_height = Some(self.font_size * sy);
            }
        }

        // Colors from first glyph
        self.stroke_color = ch.stroke_color().map(|c| color_to_argb_hex(&c));
        self.fill_color = ch.fill_color().map(|c| color_to_argb_hex(&c));

        // Marked content from first glyph
        self.mcid = ch.marked_content_id();

        // Check codepoint for buggy encoding
        let unicode = ch.unicode();
        if !self.font_is_buggy && self.font_is_embedded && is_buggy_codepoint(unicode) {
            self.font_is_buggy = true;
        }
    }

    /// Add a visible character to the current segment.
    fn push_char(
        &mut self,
        c: char,
        vp_loose: &RectF,
        vp_strict: &RectF,
        ch: &pdfium::TextChar,
        recovered: bool,
    ) {
        self.text.push(c);
        self.add_word_char(c, vp_loose);
        self.vp_left = self.vp_left.min(vp_loose.left);
        self.vp_right = self.vp_right.max(vp_loose.right);
        self.vp_top = self.vp_top.min(vp_loose.top);
        self.vp_bottom = self.vp_bottom.max(vp_loose.bottom);
        self.last_char_right = vp_strict.right;
        self.last_char_loose_right = vp_loose.right;
        self.last_char_left = vp_strict.left;
        self.last_char_loose_left = vp_loose.left;
        self.last_char_bottom = vp_strict.bottom;
        self.note_direction(c);
        self.char_count += 1;
        if self.extract_text_metadata {
            self.char_codes.push(ch.char_code());
        }
        if counts_as_unmapped(recovered, ch.has_unicode_map_error()) {
            self.unmapped_char_count += 1;
        }

        // Accumulate glyph width
        if let Some(ref font) = self.font {
            let char_code = ch.char_code();
            if ch.is_generated() {
                if let Some(w) = font.glyph_width(ch.unicode(), self.font_size) {
                    self.text_width += w;
                }
            } else if let Some(w) = font.glyph_width_from_char_code(char_code, self.font_size) {
                self.text_width += w;
            }
        }

        // Check codepoint for buggy encoding on subsequent chars
        if !self.font_is_buggy && self.font_is_embedded {
            let unicode = ch.unicode();
            if is_buggy_codepoint(unicode) {
                self.font_is_buggy = true;
            }
        }
    }

    /// Append extra characters to the segment text (for ligature expansion).
    /// Does not update bounding boxes or char count.
    fn append_ligature_tail(&mut self, tail: &str) {
        self.text.push_str(tail);
        if self.word_has {
            self.cur_word.push_str(tail);
        }
    }

    /// Returns true if the segment contains any characters that aren't dots or spaces.
    fn has_non_dot_content(&self) -> bool {
        self.text
            .chars()
            .any(|c| c != '.' && c != ' ' && c != '·' && c != '•')
    }

    /// Record that a space was seen.
    fn mark_pending_space(&mut self, is_generated: bool, char_code: u32) {
        if self.has_content {
            self.pending_space = true;
            if self.extract_text_metadata {
                self.pending_space_generated = is_generated;
                self.pending_space_char_codes.push(char_code);
            }
        }
    }

    /// Commit a pending space into the segment text.
    fn commit_pending_space(&mut self) {
        if self.pending_space {
            self.break_word();
            self.text.push(' ');
            if self.extract_text_metadata {
                self.char_codes.append(&mut self.pending_space_char_codes);
            }
            self.pending_space = false;
            self.pending_space_generated = false;
        }
    }

    /// Flush the current segment into the items list and reset.
    fn flush(&mut self, items: &mut Vec<TextItem>) {
        if !self.has_content {
            return;
        }

        self.break_word();
        if self.extract_text_metadata {
            self.char_codes.append(&mut self.pending_space_char_codes);
        }
        let trimmed = self.text.trim();
        if !trimmed.is_empty() {
            let width = self.vp_right - self.vp_left;
            let height = self.vp_bottom - self.vp_top;

            items.push(TextItem {
                text: trimmed.to_string(),
                x: self.vp_left,
                y: self.vp_top,
                width,
                height,
                rotation: self.rotation_deg,
                font_name: self.font_name.clone(),
                font_size: Some(if self.font_size > 0.0 {
                    self.font_size
                } else {
                    height
                }),
                font_height: self.font_height,
                font_ascent: self.font_ascent,
                font_descent: self.font_descent,
                font_weight: self.font_weight,
                font_flags: self.font_flags,
                text_width: if self.text_width > 0.0 {
                    Some(self.text_width)
                } else {
                    None
                },
                font_is_buggy: self.font_is_buggy,
                // Majority vote: a stray mapped char (e.g. a space) inside an
                // otherwise unmappable Type3 run must not rescue the item.
                has_unicode_map_error: self.unmapped_char_count * 2 >= self.char_count.max(1),
                mcid: self.mcid,
                fill_color: self.fill_color.clone(),
                stroke_color: self.stroke_color.clone(),
                char_codes: std::mem::take(&mut self.char_codes),
                trailing_space_generated: self.pending_space_generated,
                confidence: None,
                link: None,
                strike: false,
                words: std::mem::take(&mut self.words),
            });
        }

        *self = Self::new(self.emit_words, self.extract_text_metadata);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn rotated_text_pdf() -> Vec<u8> {
        let content =
            b"BT /F1 10 Tf 20 40 Td (FIRSTMARK) Tj ET\nBT /F1 10 Tf 20 250 Td (SECONDMARK) Tj ET";
        let objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Rotate 90 /Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Rotate 270 /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
                content,
                b"\nendstream",
            ]
            .concat(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
                content,
                b"\nendstream",
            ]
            .concat(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn rotated_pages_use_viewport_dimensions_and_keep_edge_text() {
        let pages =
            extract_pages_from_input(&PdfInput::Bytes(rotated_text_pdf()), None, usize::MAX, None)
                .unwrap();

        assert_eq!(pages.len(), 2);
        for page in &pages {
            assert_eq!((page.page_width, page.page_height), (300.0, 200.0));
            let raw_text = page
                .text_items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<String>()
                .replace(char::is_whitespace, "");
            assert!(raw_text.contains("FIRSTMARK"), "raw text: {raw_text}");
            assert!(raw_text.contains("SECONDMARK"), "raw text: {raw_text}");
        }

        let parsed = crate::projection::project_pages_to_grid(pages);
        for page in parsed {
            let text = page
                .text_items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<String>()
                .replace(char::is_whitespace, "");
            assert!(text.contains("FIRSTMARK"), "extracted text: {text}");
            assert!(text.contains("SECONDMARK"), "extracted text: {text}");
            assert!(
                page.text_items
                    .iter()
                    .all(|item| item.x + item.width <= page.page_width + 0.1)
            );
        }
    }

    // A glyph PDFium flags with a raw /ToUnicode map error normally counts
    // toward the item's unmapped tally...
    #[test]
    fn unrecovered_map_error_counts_as_unmapped() {
        assert!(counts_as_unmapped(false, true));
    }

    // ...but once the decoder recovers it (glyph-name / reverse-cmap /
    // outline-hash resolver) the text is correct, so it must NOT count — this
    // is the fix that stops the OCR merge from discarding fully-recovered
    // buggy-font items as "unusable native".
    #[test]
    fn recovered_map_error_does_not_count_as_unmapped() {
        assert!(!counts_as_unmapped(true, true));
    }

    // A cleanly-mapped glyph never counts, recovered or not.
    #[test]
    fn cleanly_mapped_glyph_never_counts_as_unmapped() {
        assert!(!counts_as_unmapped(false, false));
        assert!(!counts_as_unmapped(true, false));
    }

    fn segment_with_one_char() -> SegmentBuilder {
        let mut segment = SegmentBuilder::new(false, true);
        segment.text = "A".into();
        segment.vp_left = 1.0;
        segment.vp_right = 7.0;
        segment.vp_top = 2.0;
        segment.vp_bottom = 12.0;
        segment.char_count = 1;
        segment.has_content = true;
        segment.char_codes = vec![65];
        segment
    }

    #[test]
    fn generated_trailing_space_is_exposed_without_changing_trimmed_text() {
        let mut segment = segment_with_one_char();
        segment.mark_pending_space(true, 32);
        let mut items = Vec::new();
        segment.flush(&mut items);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "A");
        assert_eq!(items[0].char_codes, vec![65, 32]);
        assert!(items[0].trailing_space_generated);
    }

    #[test]
    fn real_trailing_space_is_not_marked_as_generated() {
        let mut segment = segment_with_one_char();
        segment.mark_pending_space(false, 32);
        let mut items = Vec::new();
        segment.flush(&mut items);

        assert_eq!(items[0].char_codes, vec![65, 32]);
        assert!(!items[0].trailing_space_generated);
    }

    #[test]
    fn disabled_text_metadata_does_not_accumulate_source_codes() {
        let mut segment = SegmentBuilder::new(false, false);
        segment.text = "A".into();
        segment.vp_left = 1.0;
        segment.vp_right = 7.0;
        segment.vp_top = 2.0;
        segment.vp_bottom = 12.0;
        segment.char_count = 1;
        segment.has_content = true;
        segment.mark_pending_space(true, 32);
        let mut items = Vec::new();
        segment.flush(&mut items);

        assert!(items[0].char_codes.is_empty());
        assert!(!items[0].trailing_space_generated);
    }

    fn strike_item() -> TextItem {
        TextItem {
            text: "word".to_string(),
            x: 100.0,
            y: 100.0,
            width: 40.0,
            height: 10.0,
            ..Default::default()
        }
    }

    fn h_stroke(x1: f32, x2: f32, y: f32) -> GraphicPrimitive {
        GraphicPrimitive::Stroke {
            x1,
            y1: y,
            x2,
            y2: y,
            color: None,
            width: 0.5,
        }
    }

    #[test]
    fn strike_midline_stroke_detected() {
        let mut items = [strike_item()];
        // Line through the vertical middle (y≈105) spanning the item width.
        assign_strikethrough(&mut items, &[h_stroke(100.0, 140.0, 105.0)]);
        assert!(items[0].strike);
    }

    #[test]
    fn strike_underline_not_detected() {
        let mut items = [strike_item()];
        // Line near the baseline (bottom, y≈110) is an underline, not a strike.
        assign_strikethrough(&mut items, &[h_stroke(100.0, 140.0, 110.0)]);
        assert!(!items[0].strike);
    }

    #[test]
    fn strike_short_line_not_detected() {
        let mut items = [strike_item()];
        // Mid-band but only covers ~25% of the item width.
        assign_strikethrough(&mut items, &[h_stroke(100.0, 110.0, 105.0)]);
        assert!(!items[0].strike);
    }

    fn ti(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextItem {
        TextItem {
            text: text.to_string(),
            x,
            y,
            width: w,
            height: h,
            ..Default::default()
        }
    }

    #[test]
    fn dedup_drops_earlier_exact_duplicate() {
        let mut items = vec![
            ti("hello", 0.0, 0.0, 10.0, 5.0),
            ti("hello", 1.0, 0.0, 10.0, 5.0),
        ];
        dedup_overlapping_items(&mut items, false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].x, 1.0);
    }

    #[test]
    fn dedup_keeps_non_overlapping() {
        let mut items = vec![ti("a", 0.0, 0.0, 5.0, 5.0), ti("b", 100.0, 100.0, 5.0, 5.0)];
        dedup_overlapping_items(&mut items, false);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn dedup_drops_earlier_when_different_text_overlaps_heavily() {
        let mut items = vec![
            ti("old", 0.0, 0.0, 10.0, 5.0),
            ti("new", 0.0, 0.0, 10.0, 5.0),
        ];
        dedup_overlapping_items(&mut items, false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "new");
    }

    #[test]
    fn dedup_keeps_both_when_different_text_overlaps_lightly() {
        let mut items = vec![
            ti("aaa", 0.0, 0.0, 10.0, 5.0),
            ti("bbb", 9.0, 0.0, 10.0, 5.0),
        ];
        dedup_overlapping_items(&mut items, false);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn dedup_keeps_overlapping_diagonal_lines() {
        // Two stacked lines of one rotated block: their loose axis-aligned
        // bounding boxes overlap heavily, but both must survive (the
        // "Paris has the" / "eiffel tower" @51° regression).
        let mut items = vec![
            TextItem {
                rotation: 51.0,
                ..ti("Paris has the", 0.0, 0.0, 100.0, 40.0)
            },
            TextItem {
                rotation: 51.0,
                ..ti("eiffel tower", 5.0, 5.0, 100.0, 40.0)
            },
        ];
        dedup_overlapping_items(&mut items, false);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn dedup_noop_for_empty_or_single() {
        let mut empty: Vec<TextItem> = vec![];
        dedup_overlapping_items(&mut empty, false);
        assert!(empty.is_empty());
        let mut one = vec![ti("x", 0.0, 0.0, 1.0, 1.0)];
        dedup_overlapping_items(&mut one, false);
        assert_eq!(one.len(), 1);
    }

    fn rot(text: &str, rotation: f32) -> TextItem {
        TextItem {
            rotation,
            ..ti(text, 10.0, 10.0, 20.0, 5.0)
        }
    }

    fn page_with(items: Vec<TextItem>) -> LitePage {
        LitePage {
            page_number: 1,
            page_width: 100.0,
            page_height: 100.0,
            content_bounds: None,
            text_items: items,
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
            annotations: None,
            form_fields: None,
            structure_tree: None,
        }
    }

    fn texts(page: &LitePage) -> Vec<&str> {
        page.text_items.iter().map(|it| it.text.as_str()).collect()
    }

    #[test]
    fn is_diagonal_rotation_matches() {
        // Within 2° of a right angle → not diagonal.
        assert!(!is_diagonal_rotation(0.0));
        assert!(!is_diagonal_rotation(1.9));
        assert!(!is_diagonal_rotation(90.0));
        assert!(!is_diagonal_rotation(271.5));
        assert!(!is_diagonal_rotation(358.5));
        // More than 2° off → diagonal (2.57° San-francisco case, 51°, 324°).
        assert!(is_diagonal_rotation(2.57));
        assert!(is_diagonal_rotation(51.0));
        assert!(is_diagonal_rotation(324.0));
    }

    #[test]
    fn skip_diagonal_keeps_only_upright_text() {
        let mut pages = vec![page_with(vec![
            rot("upright", 0.0),
            rot("slightly-skewed", 2.57),
            rot("diagonal", 51.0),
            rot("landscape", 90.0),
        ])];
        apply_content_filters(&mut pages, None, true);
        assert_eq!(texts(&pages[0]), vec!["upright", "landscape"]);
    }

    #[test]
    fn crop_box_keeps_only_items_fully_inside_region() {
        // 100×100 page; crop away the left half (left = 0.5) → survivors must
        // sit entirely within x ∈ [50, 100].
        let cb = crate::config::CropBox {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.5,
        };
        let mut pages = vec![page_with(vec![
            ti("left", 10.0, 10.0, 20.0, 5.0),     // fully left → dropped
            ti("straddle", 45.0, 10.0, 20.0, 5.0), // crosses x=50 → dropped
            ti("right", 60.0, 10.0, 20.0, 5.0),    // fully right → kept
        ])];
        apply_content_filters(&mut pages, Some(&cb), false);
        assert_eq!(texts(&pages[0]), vec!["right"]);
    }

    #[test]
    fn content_filters_noop_without_options() {
        let mut pages = vec![page_with(vec![rot("diagonal", 45.0)])];
        apply_content_filters(&mut pages, None, false);
        assert_eq!(pages[0].text_items.len(), 1);
    }

    #[test]
    fn adjust_angle_no_rotation() {
        assert!((adjust_angle_for_rotation(0.5, 0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn adjust_angle_180() {
        let r = adjust_angle_for_rotation(PI, 2);
        assert!(r.abs() < 1e-5 || (r - 2.0 * PI).abs() < 1e-5);
    }

    #[test]
    fn adjust_angle_wraps_into_0_2pi() {
        let r = adjust_angle_for_rotation(0.0, 1);
        assert!((0.0..2.0 * PI).contains(&r));
    }

    #[test]
    fn decompose_scale_identity() {
        let m = pdfium::Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        let (sx, sy) = decompose_scale(&m);
        assert!((sx - 1.0).abs() < 1e-5);
        assert!((sy - 1.0).abs() < 1e-5);
    }

    #[test]
    fn decompose_scale_uniform() {
        let m = pdfium::Matrix {
            a: 2.0,
            b: 0.0,
            c: 0.0,
            d: 2.0,
            e: 0.0,
            f: 0.0,
        };
        let (sx, sy) = decompose_scale(&m);
        assert!((sx - 2.0).abs() < 1e-4);
        assert!((sy - 2.0).abs() < 1e-4);
    }

    #[test]
    fn buggy_font_truetype_subset_prefix() {
        assert!(is_buggy_font("TTFoo", FontType::TrueType));
        assert!(is_buggy_font("ABCDEF+TTBar", FontType::TrueType));
        assert!(!is_buggy_font("Arial", FontType::TrueType));
    }

    #[test]
    fn buggy_font_type1_underscore() {
        assert!(is_buggy_font("ABCDEF_Foo", FontType::Type1));
        assert!(!is_buggy_font("ABCDEF_Foo", FontType::TrueType));
        assert!(!is_buggy_font("Short", FontType::Type1));
    }

    #[test]
    fn buggy_codepoint_ranges() {
        assert!(is_buggy_codepoint(0x00));
        assert!(is_buggy_codepoint(0x1F));
        assert!(!is_buggy_codepoint(0x20));
        assert!(is_buggy_codepoint(0xE001));
        assert!(is_buggy_codepoint(0xF8FF));
        assert!(!is_buggy_codepoint(0xE000));
        assert!(!is_buggy_codepoint(0xF900));
        // DEL + C1 controls (0x7F-0x9F): mangled-ToUnicode signature.
        assert!(is_buggy_codepoint(0x7F));
        assert!(is_buggy_codepoint(0x80));
        assert!(is_buggy_codepoint(0x9F));
        assert!(!is_buggy_codepoint(0xA0));
    }

    #[test]
    fn color_to_argb_hex_formats() {
        let c = pdfium::Color {
            r: 0xAB,
            g: 0xCD,
            b: 0xEF,
            a: 0x12,
        };
        assert_eq!(color_to_argb_hex(&c), "12abcdef");
        let z = pdfium::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
        assert_eq!(color_to_argb_hex(&z), "00000000");
    }

    fn vector_path(
        bbox: RectF,
        segments: Vec<pdfium::PathSegment>,
        stroke: bool,
        fill: bool,
        stroke_width: f32,
        color: pdfium::Color,
    ) -> PathObject {
        PathObject {
            bbox,
            stroke_color: stroke.then_some(color),
            fill_color: fill.then_some(color),
            stroke_width,
            is_stroked: stroke,
            is_filled: fill,
            segments,
        }
    }

    #[test]
    fn vector_graphics_reports_paint_curve_and_merges_axis_lines() {
        let black = pdfium::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        let paths = vec![
            vector_path(
                RectF {
                    left: 10.0,
                    top: 10.0,
                    right: 30.0,
                    bottom: 11.0,
                },
                vec![
                    pdfium::PathSegment {
                        kind: SegmentKind::MoveTo,
                        x: 10.0,
                        y: 10.0,
                        close: false,
                    },
                    pdfium::PathSegment {
                        kind: SegmentKind::LineTo,
                        x: 20.0,
                        y: 10.0,
                        close: false,
                    },
                    pdfium::PathSegment {
                        kind: SegmentKind::BezierTo,
                        x: 20.0,
                        y: 10.0,
                        close: false,
                    },
                ],
                true,
                false,
                1.0,
                black,
            ),
            vector_path(
                RectF {
                    left: 20.0,
                    top: 10.2,
                    right: 30.0,
                    bottom: 11.0,
                },
                vec![
                    pdfium::PathSegment {
                        kind: SegmentKind::MoveTo,
                        x: 20.0,
                        y: 10.0,
                        close: false,
                    },
                    pdfium::PathSegment {
                        kind: SegmentKind::LineTo,
                        x: 30.0,
                        y: 10.0,
                        close: false,
                    },
                ],
                true,
                false,
                1.0,
                black,
            ),
        ];
        let output = build_vector_graphics(&paths, None);
        assert_eq!(output.shapes.len(), 2);
        assert!(output.shapes[0].has_curve);
        assert_eq!(output.shapes[0].stroke_color.as_deref(), Some("ff000000"));
        assert_eq!(output.lines.len(), 1);
        assert!((output.lines[0].x1 - 10.0).abs() < 0.001);
        assert!((output.lines[0].x2 - 30.0).abs() < 0.001);
        assert_eq!(output.lines[0].stroke_width, Some(1.0));
    }

    #[test]
    fn white_fill_on_content_margin_suppresses_lines_but_keeps_shape() {
        let white = pdfium::Color {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        };
        // Unstroked solid-white fill whose left edge sits on the content
        // margin: with content bounds it must contribute no lines; without
        // content bounds the heuristic is inert and the edges survive.
        let rect_segments = vec![
            pdfium::PathSegment {
                kind: SegmentKind::MoveTo,
                x: 10.0,
                y: 10.0,
                close: false,
            },
            pdfium::PathSegment {
                kind: SegmentKind::LineTo,
                x: 100.0,
                y: 10.0,
                close: false,
            },
            pdfium::PathSegment {
                kind: SegmentKind::LineTo,
                x: 100.0,
                y: 50.0,
                close: false,
            },
            pdfium::PathSegment {
                kind: SegmentKind::LineTo,
                x: 10.0,
                y: 50.0,
                close: false,
            },
        ];
        let paths = vec![vector_path(
            RectF {
                left: 10.0,
                top: 10.0,
                right: 100.0,
                bottom: 50.0,
            },
            rect_segments,
            false,
            true,
            0.0,
            white,
        )];
        let content = Rect {
            x: 10.0,
            y: 5.0,
            width: 500.0,
            height: 700.0,
        };

        let with_bounds = build_vector_graphics(&paths, Some(&content));
        assert_eq!(with_bounds.shapes.len(), 1);
        assert!(with_bounds.lines.is_empty());

        let without_bounds = build_vector_graphics(&paths, None);
        assert!(!without_bounds.lines.is_empty());
    }

    #[test]
    fn white_fill_extends_to_consecutive_overlapping_white_fill() {
        let white = pdfium::Color {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        };
        let segments = |x1: f32, x2: f32| {
            vec![
                pdfium::PathSegment {
                    kind: SegmentKind::MoveTo,
                    x: x1,
                    y: 10.0,
                    close: false,
                },
                pdfium::PathSegment {
                    kind: SegmentKind::LineTo,
                    x: x2,
                    y: 10.0,
                    close: false,
                },
            ]
        };
        // First fill sits on the left content margin; the second is drawn
        // immediately after and overlaps it, so the blank area extends.
        let paths = vec![
            vector_path(
                RectF {
                    left: 10.0,
                    top: 10.0,
                    right: 60.0,
                    bottom: 50.0,
                },
                segments(10.0, 60.0),
                false,
                true,
                0.0,
                white,
            ),
            vector_path(
                RectF {
                    left: 59.0,
                    top: 10.0,
                    right: 120.0,
                    bottom: 50.0,
                },
                segments(59.0, 120.0),
                false,
                true,
                0.0,
                white,
            ),
        ];
        let content = Rect {
            x: 10.0,
            y: 5.0,
            width: 500.0,
            height: 700.0,
        };
        let output = build_vector_graphics(&paths, Some(&content));
        assert!(output.lines.is_empty());
    }

    #[test]
    fn vector_graphics_merges_consecutive_contained_solid_fills() {
        let blue = pdfium::Color {
            r: 0,
            g: 0,
            b: 255,
            a: 255,
        };
        let paths = vec![
            vector_path(
                RectF {
                    left: 0.0,
                    top: 0.0,
                    right: 20.0,
                    bottom: 20.0,
                },
                vec![],
                false,
                true,
                0.0,
                blue,
            ),
            vector_path(
                RectF {
                    left: 2.0,
                    top: 2.0,
                    right: 10.0,
                    bottom: 10.0,
                },
                vec![],
                false,
                true,
                0.0,
                blue,
            ),
        ];
        let output = build_vector_graphics(&paths, None);
        assert_eq!(output.shapes.len(), 1);
        assert_eq!(output.shapes[0].bbox.width, 20.0);
        assert_eq!(output.shapes[0].fill_color.as_deref(), Some("ff0000ff"));
    }

    #[test]
    fn vector_graphics_ignores_unpainted_and_diagonal_paths() {
        let path = vector_path(
            RectF {
                left: 0.0,
                top: 0.0,
                right: 10.0,
                bottom: 10.0,
            },
            vec![
                pdfium::PathSegment {
                    kind: SegmentKind::MoveTo,
                    x: 0.0,
                    y: 0.0,
                    close: false,
                },
                pdfium::PathSegment {
                    kind: SegmentKind::LineTo,
                    x: 10.0,
                    y: 10.0,
                    close: false,
                },
            ],
            false,
            false,
            0.0,
            pdfium::Color::default(),
        );
        let output = build_vector_graphics(&[path], None);
        assert!(output.shapes.is_empty());
        assert!(output.lines.is_empty());
    }

    #[test]
    fn extract_pages_from_input_missing_file_errors() {
        let res = extract_pages_from_input(
            &PdfInput::Path("/nonexistent/path/does-not-exist.pdf".to_string()),
            None,
            usize::MAX,
            None,
        );
        assert!(res.is_err());
    }

    #[test]
    fn page_error_is_fail_fast_by_default() {
        let mut page_errors = Vec::new();
        let result = resolve_page_result::<()>(
            3,
            Err(LiteParseError::Other("broken page".into())),
            false,
            &mut page_errors,
        );

        assert!(result.is_err());
        assert!(page_errors.is_empty());
    }

    #[test]
    fn page_error_can_be_collected_and_skipped() {
        let mut page_errors = Vec::new();
        let result = resolve_page_result::<()>(
            3,
            Err(LiteParseError::Other("broken page".into())),
            true,
            &mut page_errors,
        )
        .unwrap();

        assert!(result.is_none());
        assert_eq!(
            page_errors,
            vec![PageError {
                page_number: 3,
                message: "broken page".into(),
            }]
        );
    }

    #[test]
    fn image_cache_remove_page_drops_failed_pages_renders() {
        let image_ref = |id: &str, raw: &[u8]| ImageRef {
            id: id.to_string(),
            bbox: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            obj_index: 0,
            format: "png".into(),
            pixel_width: 1,
            pixel_height: 1,
            rotation: 0.0,
            jpeg_bytes: None,
            raw_bytes: Some(raw.to_vec()),
            bits_per_pixel: 32,
            colorspace: 0,
        };
        let cached = |id: &str, page: u32, raw: &[u8]| CachedImage {
            raw_bytes: raw.to_vec(),
            id: id.to_string(),
            page,
            format: "png".into(),
            bytes: std::sync::Arc::new(Vec::new()),
        };

        let mut cache = ImageCache::default();
        cache.insert(&image_ref("p2_1", b"two"), cached("p2_1", 2, b"two"));
        cache.insert(&image_ref("p3_1", b"three"), cached("p3_1", 3, b"three"));

        // Page 2 failed after rendering: its cache entry must go so a later
        // duplicate of the same bytes can't claim `duplicate_of: "p2_1"` for
        // an image that was rolled back out of the output.
        cache.remove_page(2);

        assert!(cache.get(&image_ref("p5_1", b"two"), b"two").is_none());
        assert_eq!(
            cache
                .get(&image_ref("p5_2", b"three"), b"three")
                .map(|c| c.id.as_str()),
            Some("p3_1")
        );
    }
}
