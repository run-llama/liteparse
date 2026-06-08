use crate::error::LiteParseError;
use crate::types::{Page as LitePage, PdfInput, TextItem};
use pdfium::{Document, Font, FontType, Library, Page, RectF, TextPage};

/// Open a PDF from path or bytes with an optional password.
pub(crate) fn load_document_from_input(
    input: &PdfInput,
    password: Option<&str>,
) -> Result<Document, LiteParseError> {
    let lib = Library::init();
    match input {
        PdfInput::Path(path) => Ok(lib.load_document(path, password)?),
        PdfInput::Bytes(data) => Ok(lib.load_document_from_bytes(data, password)?),
    }
}

/// Extract pages from a `PdfInput` (file path or bytes) with filtering.
pub fn extract_pages_from_input(
    input: &PdfInput,
    target_pages: Option<&[u32]>,
    max_pages: usize,
    password: Option<&str>,
) -> Result<Vec<LitePage>, LiteParseError> {
    let document = load_document_from_input(input, password)?;
    extract_pages_from_document(&document, target_pages, max_pages)
}

/// Extract pages from an already-open PDFium document.
pub(crate) fn extract_pages_from_document(
    document: &Document,
    target_pages: Option<&[u32]>,
    max_pages: usize,
) -> Result<Vec<LitePage>, LiteParseError> {
    let page_count = document.page_count();
    let mut pages = Vec::new();

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
        let text_items = extract_page_text_items(&page, &text_page, &view_box)?;

        pages.push(LitePage {
            page_number: page_number as usize,
            page_width: page.width(),
            page_height: page.height(),
            text_items,
            images: Vec::new(),
        });
    }

    Ok(pages)
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
) -> Result<Vec<TextItem>, LiteParseError> {
    let char_count = text_page.char_count();
    if char_count <= 0 {
        return Ok(Vec::new());
    }

    // Hard limit: gaps larger than this always cause a split (column breaks).
    const MAX_INLINE_GAP: f32 = 15.0;

    let debug = std::env::var("LITEPARSE_DEBUG").is_ok();

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

    for i in 0..char_count {
        let ch = text_page.char_at_unchecked(i);
        let unicode = ch.unicode();
        let is_generated = ch.is_generated();

        // Skip null / invalid sentinels
        if unicode == 0 || unicode == 0xFFFE || unicode == 0xFFFF {
            if debug {
                eprintln!("[extract-debug] i={i} SKIP sentinel unicode=0x{unicode:04X}");
            }
            continue;
        }

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

        // Map to a Rust char, with special-case replacements.
        // Some PDF fonts encode ligatures as control characters; expand them.
        // We use the first char for segment decisions, then append trailing chars.
        let (c, ligature_tail): (char, &str) = match unicode {
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
        };

        // Newlines: flush the current segment
        if c == '\n' || c == '\r' {
            seg.flush(&mut items);
            continue;
        }

        // Spaces: mark that we're in a pending-space state.
        if c == ' ' {
            seg.mark_pending_space();
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
                // are spaced apart, unlike periods in abbreviations like "U.S.")
                c == '.' && seg.has_non_dot_content() && gap > seg.avg_char_width() * 0.4
            };

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
                    seg.commit_pending_space();
                    seg.push_char(c, &vp_loose, &vp_strict, &ch);
                    seg.append_ligature_tail(ligature_tail);
                }
            } else {
                seg.push_char(c, &vp_loose, &vp_strict, &ch);
                seg.append_ligature_tail(ligature_tail);
            }
        } else {
            seg.start(c, &vp_loose, &vp_strict, &ch, page_rotation);
            seg.append_ligature_tail(ligature_tail);
        }
    }

    seg.flush(&mut items);

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
                // Exact text match: any overlap → drop the earlier item
                // (later items are rendered on top in PDF paint order)
                if debug {
                    eprintln!(
                        "[extract-debug] DEDUP exact-match drop i={i} text='{}' at ({:.1},{:.1}) in favor of j={j} at ({:.1},{:.1})",
                        items[i].text, items[i].x, items[i].y, items[j].x, items[j].y
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
    // Bottom of last char strict bounds (for line-change detection)
    last_char_bottom: f32,
    // Count of non-space characters (for avg width calculation)
    char_count: usize,
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
            last_char_bottom: f32::MIN,
            char_count: 0,
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
        self.last_char_bottom = vp_strict.bottom;
        self.char_count = 1;
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
        self.last_char_bottom = vp_strict.bottom;
        self.char_count += 1;

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
                mcid: self.mcid,
                fill_color: self.fill_color.clone(),
                stroke_color: self.stroke_color.clone(),
                confidence: None,
            });
        }

        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

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
        );
        assert!(res.is_err());
    }
}
