use crate::error::LiteParseError;
use crate::glyph_names::resolve_glyph_name;
use crate::types::{
    ExtractedImage, GraphicPrimitive, ImageRef, OutlineTarget, Page as LitePage, PdfInput, Rect,
    StructNode, TextItem,
};
use image::ImageEncoder;
use pdfium::{
    Document, Font, FontType, Library, Page, PathObject, PdfLink, RectF, SegmentKind, TextPage,
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
    word_granularity: bool,
) -> Result<Vec<LitePage>, LiteParseError> {
    let lib = Library::init();
    let document = load_document_from_input(&lib, input, password)?;
    extract_pages_from_document(&document, target_pages, max_pages, word_granularity)
}

/// Extract pages from an already-open PDFium document.
pub(crate) fn extract_pages_from_document(
    document: &Document,
    target_pages: Option<&[u32]>,
    max_pages: usize,
    word_granularity: bool,
) -> Result<Vec<LitePage>, LiteParseError> {
    Ok(extract_pages_and_images(
        document,
        target_pages,
        max_pages,
        false,
        false,
        word_granularity,
    )?
    .0)
}

/// Same as `extract_pages_from_document` but optionally also renders every
/// raster image object to PNG bytes (when `render_images = true`). Returned
/// `ExtractedImage`s carry the same ids the markdown emitter will reference,
/// so callers can match them up by id. When `render_images = false` the
/// returned image vec is always empty.
pub(crate) fn extract_pages_and_images(
    document: &Document,
    target_pages: Option<&[u32]>,
    max_pages: usize,
    render_images: bool,
    extract_links: bool,
    word_granularity: bool,
) -> Result<(Vec<LitePage>, Vec<ExtractedImage>), LiteParseError> {
    let page_count = document.page_count();
    let mut pages = Vec::new();
    let mut images: Vec<ExtractedImage> = Vec::new();

    for page_index in 0..page_count {
        let page_number = page_index as u32 + 1;

        if let Some(targets) = target_pages
            && !targets.contains(&page_number)
        {
            continue;
        }

        if pages.len() >= max_pages {
            break;
        }

        let page = document.page(page_index)?;
        let text_page = page.text()?;
        let view_box = page.view_box().unwrap_or(RectF {
            left: 0.0,
            top: page.height(),
            right: page.width(),
            bottom: 0.0,
        });
        let mut text_items =
            extract_page_text_items(&page, &text_page, &view_box, word_granularity)?;
        if extract_links {
            assign_links(&mut text_items, &page.links(&view_box));
        }
        let graphics = extract_page_graphics(&page, &view_box);
        assign_strikethrough(&mut text_items, &graphics);
        let struct_nodes = extract_page_struct_nodes(&page, &view_box);
        let image_refs = extract_page_image_refs(&page, page_number);

        if render_images && !image_refs.is_empty() {
            images.extend(render_page_images(&page, page_number, &image_refs));
        }

        pages.push(LitePage {
            page_number: page_number as usize,
            page_width: page.width(),
            page_height: page.height(),
            text_items,
            graphics,
            struct_nodes,
            image_refs,
        });
    }

    Ok((pages, images))
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
pub fn extract(
    pdf_path: &str,
    page_num: Option<u32>,
    word_granularity: bool,
) -> Result<(), LiteParseError> {
    let target_pages: Option<Vec<u32>> = page_num.map(|p| vec![p]);
    let pages = extract_pages_from_input(
        &PdfInput::Path(pdf_path.to_string()),
        target_pages.as_deref(),
        usize::MAX,
        None,
        word_granularity,
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

/// Render every image referenced in `refs` to PNG bytes using
/// `Page::render_image_object`. Returns one `ExtractedImage` per ref. Used by
/// the parser only when `ImageMode::Embed` is configured — otherwise the
/// extraction loop skips this entirely. Failures for individual images are
/// silently dropped (a malformed embedded image shouldn't fail the whole
/// parse).
pub(crate) fn render_page_images(
    page: &Page,
    page_number: u32,
    refs: &[ImageRef],
) -> Vec<ExtractedImage> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let bmp = match page.render_image_object(r.obj_index) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let w = bmp.width().max(0) as u32;
        let h = bmp.height().max(0) as u32;
        if w == 0 || h == 0 {
            continue;
        }
        let rgba = bmp.to_rgba();
        let png = match encode_png(&rgba, w, h) {
            Ok(p) => p,
            Err(_) => continue,
        };
        out.push(ExtractedImage {
            id: r.id.clone(),
            page: page_number,
            bbox: r.bbox.clone(),
            format: "png".into(),
            bytes: png,
        });
    }
    out
}

/// Encode RGBA pixel bytes to PNG. Lives here (always-compiled) rather than in
/// `render` so the image-embed path is available on wasm, where the `render`
/// module (page rasterization / screenshots) is compiled out.
pub(crate) fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, LiteParseError> {
    let mut png_buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
    encoder.write_image(rgba, width, height, image::ColorType::Rgba8.into())?;
    Ok(png_buf)
}

/// Walk image objects on a page and return a stable per-page `ImageRef` for
/// each one. `obj_index` is the index among image-typed page objects (not all
/// page objects), so a later embed pass can pull pixel bytes via
/// `Page::render_image_object`. IDs are scoped to the page number so they
/// remain stable across runs.
fn extract_page_image_refs(page: &Page, page_number: u32) -> Vec<ImageRef> {
    page.image_bounds(IMAGE_MIN_SIZE_PT, IMAGE_MAX_COVERAGE)
        .into_iter()
        .enumerate()
        .map(|(i, b)| ImageRef {
            id: format!("p{}_{}", page_number, i),
            bbox: Rect {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            },
            obj_index: i,
        })
        .collect()
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
fn extract_page_graphics(page: &Page, view_box: &RectF) -> Vec<GraphicPrimitive> {
    let paths: Vec<PathObject> = page.path_objects(view_box);
    let mut out = Vec::new();

    for path in &paths {
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
    word_granularity: bool,
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
    let mut seg = SegmentBuilder::new();
    let garbage_fonts = detect_garbage_unicode_fonts(text_page, char_count);
    let mut glyph_decoder = GlyphDecoder::new(
        std::env::var("LITEPARSE_DEBUG_GLYPH").is_ok(),
        garbage_fonts,
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

        // Spaces: mark that we're in a pending-space state.
        if c == ' ' {
            if word_granularity {
                if seg.has_content {
                    seg.flush(&mut items);
                }
            } else {
                seg.mark_pending_space();
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

            let gap = vp_strict.left - seg.last_char_right;

            // Detect line change using complementary checks:
            // 1. Strict vertical separation: char's strict top is well below last char's strict bottom
            // 2. Line wrap: char goes back leftward AND strict top is below last char's strict bottom
            //    (even slightly), indicating text wrapped to a new line within the same text object
            // 3. Very large leftward jump: if the char jumps back by more than the current
            //    segment width, it's definitely a new line (handles OCR text with tall bounding
            //    boxes that overlap vertically between lines)
            let strict_below = vp_strict.top > seg.last_char_bottom;
            let large_leftward_jump = gap < -5.0;
            let seg_width = seg.vp_right - seg.vp_left;
            let very_large_leftward_jump = seg_width > 20.0 && gap < -(seg_width * 0.5);
            let line_changed = vp_strict.top > seg.last_char_bottom + y_tolerance
                || (strict_below && large_leftward_jump)
                || very_large_leftward_jump;

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
                let loose_gap = vp_strict.left - seg.last_char_loose_right;
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
                seg.start(c, &vp_loose, &vp_strict, &ch, page_rotation);
                seg.append_ligature_tail(ligature_tail);
            } else if seg.pending_space {
                let avg_cw = seg.avg_char_width();
                if gap > avg_cw * 2.2 {
                    seg.flush(&mut items);
                    seg.start(c, &vp_loose, &vp_strict, &ch, page_rotation);
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
                            let loose_gap = vp_strict.left - seg.last_char_loose_right;
                            if em_vp > 0.0 && loose_gap > 0.0 {
                                let s = font_space_cal.entry(fk.clone()).or_default();
                                if s.len() < 512 {
                                    s.push(loose_gap / em_vp);
                                }
                            }
                        }
                    }
                    seg.commit_pending_space();
                    seg.push_char(c, &vp_loose, &vp_strict, &ch);
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
                let loose_gap = vp_strict.left - seg.last_char_loose_right;
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
                }
                seg.push_char(c, &vp_loose, &vp_strict, &ch);
                seg.append_ligature_tail(ligature_tail);
            }
        } else {
            seg.start(c, &vp_loose, &vp_strict, &ch, page_rotation);
            seg.append_ligature_tail(ligature_tail);
        }
    }

    seg.flush(&mut items);

    // Drop items entirely outside the page view box. Print-spread / imposed
    // PDFs carry the neighbouring page's text at x beyond the page edge in
    // the same content stream; viewers never show it. Partially-visible
    // items are kept.
    let vb_w = (view_box.right - view_box.left).abs();
    let vb_h = (view_box.top - view_box.bottom).abs();
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
/// Computes eigenvalues of M^T * M, matching the platform's Parse_decomposeScale.
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
/// Mirrors ParseFont_isBuggyFont from the platform.
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
fn is_buggy_codepoint(unicode: u32) -> bool {
    unicode <= 0x1F || (unicode > 0xE000 && unicode <= 0xF8FF)
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
struct GlyphDecoder {
    fonts: std::collections::HashMap<usize, FontGlyphInfo>,
    /// Chars arrive in runs per text object; cache the last object's font key
    /// to skip the FPDFTextObj_GetFont FFI call on the common path.
    last_obj: usize,
    last_key: usize,
    /// Font handles whose /ToUnicode the prescan flagged as garbage (high
    /// fraction of control/PUA unicodes across the page).
    garbage_fonts: std::collections::HashSet<usize>,
    debug: bool,
}

struct FontGlyphInfo {
    font: Font,
    /// No /ToUnicode and no standard base encoding (or the prescan flagged
    /// the ToUnicode as garbage): PDFium's unicode values for this font are
    /// untrusted, so every charcode gets a recovery try.
    untrusted: bool,
    /// charcode → resolved replacement text (None = unrecoverable)
    cache: std::collections::HashMap<u32, Option<String>>,
    /// Lazily-built glyph_index → unicode map from the embedded font
    /// program's cmap table (None = not yet built, Some(None) = unavailable).
    reverse_cmap: Option<Option<std::collections::HashMap<u32, u32>>>,
}

impl GlyphDecoder {
    fn new(debug: bool, garbage_fonts: std::collections::HashSet<usize>) -> Self {
        Self {
            fonts: std::collections::HashMap::new(),
            garbage_fonts,
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
                let untrusted = garbage
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
                        "[glyph] font={:?} to_unicode={} encoding={:?} garbage={} untrusted={}",
                        font.base_name(),
                        has_to_unicode,
                        encoding,
                        garbage,
                        untrusted
                    );
                }
                FontGlyphInfo {
                    font,
                    untrusted,
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

        let char_code = ch.char_code();
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
                let resolved = name
                    .as_deref()
                    .and_then(resolve_glyph_name)
                    .filter(|r| r.chars().all(|c| !c.is_control()));
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
    has_content: bool,
    pending_space: bool,
}

impl SegmentBuilder {
    fn new() -> Self {
        Self {
            text: String::new(),
            vp_left: f32::MAX,
            vp_right: f32::MIN,
            vp_top: f32::MAX,
            vp_bottom: f32::MIN,
            last_char_right: f32::MIN,
            last_char_loose_right: f32::MIN,
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
            has_content: false,
            pending_space: false,
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
        self.last_char_bottom = vp_strict.bottom;
        self.char_count = 1;
        self.unmapped_char_count = if ch.has_unicode_map_error() { 1 } else { 0 };
        self.has_content = true;
        self.pending_space = false;
        self.text_width = 0.0;
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
    fn push_char(&mut self, c: char, vp_loose: &RectF, vp_strict: &RectF, ch: &pdfium::TextChar) {
        self.text.push(c);
        self.vp_left = self.vp_left.min(vp_loose.left);
        self.vp_right = self.vp_right.max(vp_loose.right);
        self.vp_top = self.vp_top.min(vp_loose.top);
        self.vp_bottom = self.vp_bottom.max(vp_loose.bottom);
        self.last_char_right = vp_strict.right;
        self.last_char_loose_right = vp_loose.right;
        self.last_char_bottom = vp_strict.bottom;
        self.char_count += 1;
        if ch.has_unicode_map_error() {
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
    }

    /// Returns true if the segment contains any characters that aren't dots or spaces.
    fn has_non_dot_content(&self) -> bool {
        self.text
            .chars()
            .any(|c| c != '.' && c != ' ' && c != '·' && c != '•')
    }

    /// Record that a space was seen.
    fn mark_pending_space(&mut self) {
        if self.has_content {
            self.pending_space = true;
        }
    }

    /// Commit a pending space into the segment text.
    fn commit_pending_space(&mut self) {
        if self.pending_space {
            self.text.push(' ');
            self.pending_space = false;
        }
    }

    /// Flush the current segment into the items list and reset.
    fn flush(&mut self, items: &mut Vec<TextItem>) {
        if !self.has_content {
            return;
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
                confidence: None,
                link: None,
                strike: false,
            });
        }

        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

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
    fn dedup_noop_for_empty_or_single() {
        let mut empty: Vec<TextItem> = vec![];
        dedup_overlapping_items(&mut empty, false);
        assert!(empty.is_empty());
        let mut one = vec![ti("x", 0.0, 0.0, 1.0, 1.0)];
        dedup_overlapping_items(&mut one, false);
        assert_eq!(one.len(), 1);
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

    #[test]
    fn extract_pages_from_input_missing_file_errors() {
        let res = extract_pages_from_input(
            &PdfInput::Path("/nonexistent/path/does-not-exist.pdf".to_string()),
            None,
            usize::MAX,
            None,
            false,
        );
        assert!(res.is_err());
    }
}
