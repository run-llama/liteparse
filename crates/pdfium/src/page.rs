use std::marker::PhantomData;

use crate::bitmap::Bitmap;
use crate::document::Document;
use crate::error::PdfiumError;
use crate::ffi;
use crate::text_page::TextPage;
use crate::types::{Color, RectF};

/// Bounding box of an embedded image object on a page.
/// Coordinates are in PDF points with top-left origin (Y-down).
#[derive(Debug, Clone, Copy)]
pub struct ImageBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

fn image_object_data(obj: pdfium_sys::FPDF_PAGEOBJECT, decoded: bool) -> Option<Vec<u8>> {
    let size = unsafe {
        if decoded {
            ffi!(FPDFImageObj_GetImageDataDecoded(
                obj,
                std::ptr::null_mut(),
                0
            ))
        } else {
            ffi!(FPDFImageObj_GetImageDataRaw(obj, std::ptr::null_mut(), 0))
        }
    };
    if size == 0 || size > usize::MAX as std::os::raw::c_ulong {
        return None;
    }
    let mut bytes = vec![0u8; size as usize];
    let written = unsafe {
        if decoded {
            ffi!(FPDFImageObj_GetImageDataDecoded(
                obj,
                bytes.as_mut_ptr().cast(),
                size
            ))
        } else {
            ffi!(FPDFImageObj_GetImageDataRaw(
                obj,
                bytes.as_mut_ptr().cast(),
                size
            ))
        }
    };
    if written == 0 || written > size {
        return None;
    }
    bytes.truncate(written as usize);
    Some(bytes)
}

fn image_filters(obj: pdfium_sys::FPDF_PAGEOBJECT) -> Vec<String> {
    let count = unsafe { ffi!(FPDFImageObj_GetImageFilterCount(obj)) };
    if count <= 0 {
        return Vec::new();
    }
    (0..count)
        .filter_map(|index| {
            let size = unsafe {
                ffi!(FPDFImageObj_GetImageFilter(
                    obj,
                    index,
                    std::ptr::null_mut(),
                    0
                ))
            };
            if size == 0 || size > 256 {
                return None;
            }
            let mut bytes = vec![0u8; size as usize];
            let written = unsafe {
                ffi!(FPDFImageObj_GetImageFilter(
                    obj,
                    index,
                    bytes.as_mut_ptr().cast(),
                    size
                ))
            };
            if written == 0 {
                return None;
            }
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            std::str::from_utf8(&bytes[..end]).ok().map(str::to_owned)
        })
        .collect()
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.ends_with(&[0xff, 0xd9])
}

#[cfg(test)]
mod image_tests {
    use super::is_jpeg;

    #[test]
    fn validates_complete_jpeg_streams() {
        assert!(is_jpeg(&[0xff, 0xd8, 0xff, 0xe0, 1, 2, 0xff, 0xd9]));
        assert!(!is_jpeg(&[0xff, 0xd8, 0xff, 0xe0]));
        assert!(!is_jpeg(&[0x89, b'P', b'N', b'G', 0xff, 0xd9]));
    }
}

/// Metadata for an embedded image page object retained by the extraction
/// filters. `object_index` is its index among all image objects on the page.
#[derive(Debug, Clone)]
pub struct ImageObjectInfo {
    pub object_index: usize,
    pub bounds: ImageBounds,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Clockwise page-object rotation in degrees, normalized to `[0, 360)`.
    pub rotation: f32,
    /// Original JPEG stream bytes when PDFium reports a directly decodable
    /// DCT stream and the decoded data has a valid JPEG signature.
    pub jpeg_bytes: Option<Vec<u8>>,
    /// Raw encoded stream bytes, used to identify repeated image resources.
    pub raw_bytes: Option<Vec<u8>>,
    #[doc(hidden)]
    pub bits_per_pixel: u32,
    #[doc(hidden)]
    pub colorspace: i32,
}

#[derive(Debug, Clone)]
pub struct ImageObjects {
    pub images: Vec<ImageObjectInfo>,
    pub error_count: u32,
}

/// One segment of a vector path. Coordinates are in viewport space
/// (top-left origin, 72 DPI) after the object's matrix has been applied.
#[derive(Debug, Clone, Copy)]
pub struct PathSegment {
    pub kind: SegmentKind,
    pub x: f32,
    pub y: f32,
    /// Whether this segment closes the current subpath back to its MoveTo.
    pub close: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    MoveTo,
    LineTo,
    BezierTo,
}

/// A vector path object extracted from a page. Used by the markdown emitter
/// for ruled-table, horizontal-rule, and figure-cluster detection.
#[derive(Debug, Clone)]
pub struct PathObject {
    /// Object bbox in viewport space (after matrix; from FPDFPageObj_GetBounds).
    pub bbox: RectF,
    pub stroke_color: Option<Color>,
    pub fill_color: Option<Color>,
    pub stroke_width: f32,
    /// True when the path is stroked per its draw mode.
    pub is_stroked: bool,
    /// True when the path is filled (draw-mode fill ≠ NONE).
    pub is_filled: bool,
    pub segments: Vec<PathSegment>,
}

/// A URI hyperlink annotation on a page. `rect` is in viewport space
/// (top-left origin, 72 DPI), matching `TextItem` coordinates so the URI can
/// be assigned to overlapping text. Only external URI links are represented;
/// internal GoTo/named destinations are excluded.
#[derive(Debug, Clone)]
pub struct PdfLink {
    pub rect: RectF,
    pub uri: String,
}

/// A loaded page within a [`Document`].
///
/// The `'doc` lifetime ties the page to its owning document; `'lib` carries
/// the PDFium-lock lifetime through, ensuring no PDFium calls can occur
/// after the lock is released.
pub struct Page<'doc, 'lib: 'doc> {
    pub(crate) handle: pdfium_sys::FPDF_PAGE,
    pub(crate) doc_handle: pdfium_sys::FPDF_DOCUMENT,
    pub(crate) _doc: PhantomData<&'doc Document<'lib>>,
}

impl<'doc, 'lib: 'doc> Page<'doc, 'lib> {
    pub fn width(&self) -> f32 {
        unsafe { ffi!(FPDF_GetPageWidthF(self.handle)) }
    }

    pub fn height(&self) -> f32 {
        unsafe { ffi!(FPDF_GetPageHeightF(self.handle)) }
    }

    pub fn rotation(&self) -> i32 {
        unsafe { ffi!(FPDFPage_GetRotation(self.handle)) }
    }

    /// Get the page bounding box (CropBox, falls back to MediaBox).
    /// Coordinates in PDF page space.
    pub fn view_box(&self) -> Option<RectF> {
        let mut rect = pdfium_sys::FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        let ok = unsafe { ffi!(FPDF_GetPageBoundingBox(self.handle, &mut rect)) };
        if ok != 0 {
            Some(RectF {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            })
        } else {
            None
        }
    }

    /// Convert a point from PDF page space to viewport space (top-left origin, 72 DPI).
    /// Mirrors the platform's Parse_pageToViewport using FPDF_PageToDevice at 1000x scale.
    pub fn page_to_viewport(&self, view_box: &RectF, page_x: f32, page_y: f32) -> (f32, f32) {
        let mut vw = view_box.right - view_box.left;
        let mut vh = view_box.top - view_box.bottom;

        let rotation = self.rotation();
        if rotation == 1 || rotation == 3 {
            // 90° or 270° — swap viewport dimensions
            std::mem::swap(&mut vw, &mut vh);
        }

        let device_w = (vw * 1000.0).round() as i32;
        let device_h = (vh * 1000.0).round() as i32;
        let mut dx: i32 = 0;
        let mut dy: i32 = 0;

        unsafe {
            ffi!(FPDF_PageToDevice(
                self.handle,
                0,
                0,
                device_w,
                device_h,
                0, // rotation 0 — PDFium applies page rotation internally
                page_x as f64,
                page_y as f64,
                &mut dx,
                &mut dy,
            ));
        }

        (dx as f32 / 1000.0, dy as f32 / 1000.0)
    }

    /// Convert bounds from PDF page space to viewport space (top-left origin).
    /// Returns RectF with left/top/right/bottom in viewport coordinates.
    pub fn bounds_to_viewport(&self, view_box: &RectF, page_bounds: &RectF) -> RectF {
        let (ll_x, ll_y) = self.page_to_viewport(view_box, page_bounds.left, page_bounds.bottom);
        let (ur_x, ur_y) = self.page_to_viewport(view_box, page_bounds.right, page_bounds.top);

        RectF {
            left: ll_x.min(ur_x),
            top: ll_y.min(ur_y),
            right: ll_x.max(ur_x),
            bottom: ll_y.max(ur_y),
        }
    }

    pub fn text(&self) -> Result<TextPage<'_, 'lib>, PdfiumError> {
        let handle = unsafe { ffi!(FPDFText_LoadPage(self.handle)) };
        if handle.is_null() {
            return Err(PdfiumError::OperationFailed);
        }
        Ok(TextPage {
            handle,
            _page: PhantomData,
        })
    }

    /// Render the page to a BGRA bitmap at the given DPI.
    pub fn render(&self, dpi: f32) -> Result<Bitmap<'lib>, PdfiumError> {
        let scale = dpi / 72.0;
        let width = (self.width() * scale).round() as i32;
        let height = (self.height() * scale).round() as i32;

        // SAFETY: this method is on `Page<'_, 'lib>`, whose existence proves
        // the PDFium lock is held for `'lib`; the returned `Bitmap<'lib>` is
        // tied to that same lock lifetime.
        let bitmap = unsafe { Bitmap::new(width, height) }?;

        // Fill with white (ARGB: 0xFFFFFFFF)
        bitmap.fill_rect(0, 0, width, height, 0xFFFFFFFF);

        let flags = (pdfium_sys::FPDF_ANNOT | pdfium_sys::FPDF_PRINTING) as i32;

        unsafe {
            ffi!(FPDF_RenderPageBitmap(
                bitmap.handle(),
                self.handle,
                0,      // start_x
                0,      // start_y
                width,  // size_x
                height, // size_y
                0,      // rotation
                flags,
            ));
        }

        Ok(bitmap)
    }

    /// Extract bounding boxes of embedded image objects on this page.
    /// Returns coordinates in viewport space (Y-down, top-left origin) in PDF points.
    /// Filters out images smaller than `min_size_pt` and images covering more than
    /// `max_page_coverage` fraction of the page.
    pub fn image_bounds(&self, min_size_pt: f32, max_page_coverage: f32) -> Vec<ImageBounds> {
        self.image_objects(min_size_pt, max_page_coverage, false)
            .images
            .into_iter()
            .map(|image| image.bounds)
            .collect()
    }

    /// Extract metadata for embedded image objects on this page.
    pub fn image_objects(
        &self,
        min_size_pt: f32,
        max_page_coverage: f32,
        include_data: bool,
    ) -> ImageObjects {
        let page_width = self.width();
        let page_height = self.height();
        let view_box = self.view_box().unwrap_or(RectF {
            left: 0.0,
            top: page_height,
            right: page_width,
            bottom: 0.0,
        });
        let obj_count = unsafe { ffi!(FPDFPage_CountObjects(self.handle)) };
        let mut results = Vec::new();
        let mut error_count = 0;
        let mut image_index = 0usize;

        for i in 0..obj_count {
            let obj = unsafe { ffi!(FPDFPage_GetObject(self.handle, i)) };
            if obj.is_null() {
                continue;
            }

            let obj_type = unsafe { ffi!(FPDFPageObj_GetType(obj)) };
            if obj_type != pdfium_sys::FPDF_PAGEOBJ_IMAGE as i32 {
                continue;
            }
            let object_index = image_index;
            image_index += 1;

            let mut left: f32 = 0.0;
            let mut bottom: f32 = 0.0;
            let mut right: f32 = 0.0;
            let mut top: f32 = 0.0;
            let ok = unsafe {
                ffi!(FPDFPageObj_GetBounds(
                    obj,
                    &mut left,
                    &mut bottom,
                    &mut right,
                    &mut top
                ))
            };
            if ok == 0 {
                error_count += 1;
                continue;
            }

            let w = right - left;
            let h = top - bottom;

            if w < min_size_pt || h < min_size_pt {
                continue;
            }
            if w > page_width * max_page_coverage && h > page_height * max_page_coverage {
                continue;
            }

            let viewport = self.bounds_to_viewport(
                &view_box,
                &RectF {
                    left,
                    top,
                    right,
                    bottom,
                },
            );

            let mut metadata = pdfium_sys::FPDF_IMAGEOBJ_METADATA::default();
            let (pixel_width, pixel_height, rotation) = if include_data {
                let metadata_ok = unsafe {
                    ffi!(FPDFImageObj_GetImageMetadata(
                        obj,
                        self.handle,
                        &mut metadata
                    ))
                };
                let mut pixel_width = metadata.width;
                let mut pixel_height = metadata.height;
                let pixel_size_ok = unsafe {
                    ffi!(FPDFImageObj_GetImagePixelSize(
                        obj,
                        &mut pixel_width,
                        &mut pixel_height
                    ))
                };
                if pixel_size_ok == 0 && metadata_ok == 0 {
                    pixel_width = 0;
                    pixel_height = 0;
                    error_count += 1;
                }

                let mut matrix = pdfium_sys::FS_MATRIX {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                };
                let matrix_ok = unsafe { ffi!(FPDFPageObj_GetMatrix(obj, &mut matrix)) };
                let rotation = if matrix_ok != 0 {
                    matrix.b.atan2(matrix.a).to_degrees().rem_euclid(360.0)
                } else {
                    0.0
                };
                (pixel_width, pixel_height, rotation)
            } else {
                (0, 0, 0.0)
            };

            let raw_bytes = include_data
                .then(|| image_object_data(obj, false))
                .flatten();
            let jpeg_bytes = if include_data
                && image_filters(obj)
                    .iter()
                    .any(|filter| filter == "DCTDecode")
            {
                image_object_data(obj, true).filter(|bytes| is_jpeg(bytes))
            } else {
                None
            };

            results.push(ImageObjectInfo {
                object_index,
                bounds: ImageBounds {
                    x: viewport.left,
                    y: viewport.top,
                    width: viewport.right - viewport.left,
                    height: viewport.bottom - viewport.top,
                },
                pixel_width,
                pixel_height,
                rotation,
                jpeg_bytes,
                raw_bytes,
                bits_per_pixel: metadata.bits_per_pixel,
                colorspace: metadata.colorspace,
            });
        }

        ImageObjects {
            images: results,
            error_count,
        }
    }

    /// Extract bounding boxes of filled vector path objects on this page,
    /// recursing into form XObjects (with each form's transform applied).
    /// Returns coordinates in viewport space (Y-down, top-left origin) in PDF
    /// points. Stroke-only paths (rules, borders) are skipped, as are paths
    /// smaller than `min_size_pt` in either dimension and paths covering more
    /// than `max_page_coverage` fraction of the page in both dimensions
    /// (full-page background rects).
    pub fn filled_path_bounds(&self, min_size_pt: f32, max_page_coverage: f32) -> Vec<ImageBounds> {
        let page_width = self.width();
        let page_height = self.height();
        let obj_count = unsafe { ffi!(FPDFPage_CountObjects(self.handle)) };
        let mut results = Vec::new();

        for i in 0..obj_count {
            let obj = unsafe { ffi!(FPDFPage_GetObject(self.handle, i)) };
            if obj.is_null() {
                continue;
            }
            collect_filled_paths(
                obj,
                None,
                page_width,
                page_height,
                min_size_pt,
                max_page_coverage,
                0,
                &mut results,
            );
        }

        results
    }

    /// Get the rendered bitmap of a specific embedded image object by index.
    /// The index corresponds to the order from iterating page objects (image objects only).
    pub fn render_image_object(&self, image_obj_index: usize) -> Result<Bitmap<'lib>, PdfiumError> {
        let obj_count = unsafe { ffi!(FPDFPage_CountObjects(self.handle)) };
        let mut image_idx = 0usize;

        for i in 0..obj_count {
            let obj = unsafe { ffi!(FPDFPage_GetObject(self.handle, i)) };
            if obj.is_null() {
                continue;
            }
            let obj_type = unsafe { ffi!(FPDFPageObj_GetType(obj)) };
            if obj_type != pdfium_sys::FPDF_PAGEOBJ_IMAGE as i32 {
                continue;
            }

            if image_idx == image_obj_index {
                let bmp_handle = unsafe {
                    ffi!(FPDFImageObj_GetRenderedBitmap(
                        self.doc_handle,
                        self.handle,
                        obj
                    ))
                };
                if bmp_handle.is_null() {
                    return Err(PdfiumError::OperationFailed);
                }
                // Wrap in our Bitmap (which will call Destroy on drop)
                return Ok(unsafe { Bitmap::from_handle(bmp_handle) });
            }
            image_idx += 1;
        }

        Err(PdfiumError::OperationFailed)
    }

    /// Enumerate vector path objects on this page. Segment points are
    /// transformed into viewport space (top-left origin, 72 DPI) by composing
    /// the object's matrix with the page→viewport transform. Recurses into
    /// Form XObjects (composing each form's matrix) — table rules and other
    /// vector art are frequently wrapped in a form container, invisible to a
    /// top-level-only walk.
    pub fn path_objects(&self, view_box: &RectF) -> Vec<PathObject> {
        let vp = self.viewport_transform(view_box);
        let obj_count = unsafe { ffi!(FPDFPage_CountObjects(self.handle)) };
        let mut out = Vec::new();
        let identity = pdfium_sys::FS_MATRIX {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };

        for i in 0..obj_count {
            let obj = unsafe { ffi!(FPDFPage_GetObject(self.handle, i)) };
            if obj.is_null() {
                continue;
            }
            collect_path_objects(obj, &identity, &vp, 0, &mut out);
        }
        out
    }

    /// Enumerate URI hyperlink annotations on this page. Each link's clickable
    /// rectangle is mapped into viewport space (matching `TextItem`); the URI
    /// is read from the link's URI action. Annotations whose action is not a
    /// URI (internal GoTo / named destinations) are skipped.
    pub fn links(&self, view_box: &RectF) -> Vec<PdfLink> {
        let mut out = Vec::new();
        let mut start_pos: std::os::raw::c_int = 0;
        let mut link_annot: pdfium_sys::FPDF_LINK = std::ptr::null_mut();
        loop {
            let ok = unsafe {
                ffi!(FPDFLink_Enumerate(
                    self.handle,
                    &mut start_pos,
                    &mut link_annot
                ))
            };
            if ok == 0 {
                break;
            }
            if link_annot.is_null() {
                continue;
            }
            let action = unsafe { ffi!(FPDFLink_GetAction(link_annot)) };
            if action.is_null() {
                continue;
            }
            let Some(uri) = read_uri_path(self.doc_handle, action) else {
                continue;
            };

            // Prefer per-line quad points: a link that wraps across lines has
            // one quad per line, each tight around the anchor text. The single
            // annotation rect is their *union* — a tall box that would swallow
            // the unlinked words sitting between the lines. Fall back to the
            // annot rect only when no quads are present.
            let quad_count = unsafe { ffi!(FPDFLink_CountQuadPoints(link_annot)) };
            let mut emitted = false;
            for q in 0..quad_count {
                let mut quad = pdfium_sys::FS_QUADPOINTSF::default();
                let ok = unsafe { ffi!(FPDFLink_GetQuadPoints(link_annot, q, &mut quad)) };
                if ok == 0 {
                    continue;
                }
                let page_bounds = RectF {
                    left: quad.x1.min(quad.x2).min(quad.x3).min(quad.x4),
                    bottom: quad.y1.min(quad.y2).min(quad.y3).min(quad.y4),
                    right: quad.x1.max(quad.x2).max(quad.x3).max(quad.x4),
                    top: quad.y1.max(quad.y2).max(quad.y3).max(quad.y4),
                };
                out.push(PdfLink {
                    rect: self.bounds_to_viewport(view_box, &page_bounds),
                    uri: uri.clone(),
                });
                emitted = true;
            }
            if emitted {
                continue;
            }

            let mut rect = pdfium_sys::FS_RECTF {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            };
            let got = unsafe { ffi!(FPDFLink_GetAnnotRect(link_annot, &mut rect)) };
            if got == 0 {
                continue;
            }
            let page_bounds = RectF {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            };
            out.push(PdfLink {
                rect: self.bounds_to_viewport(view_box, &page_bounds),
                uri,
            });
        }
        out
    }
}

/// Read a link action's URI path. PDFium returns the URI as a NUL-terminated
/// 7-bit-ASCII byte string; the two-call protocol queries the length first.
/// Returns `None` for non-URI actions (length 0) or empty URIs.
fn read_uri_path(
    doc: pdfium_sys::FPDF_DOCUMENT,
    action: pdfium_sys::FPDF_ACTION,
) -> Option<String> {
    let needed =
        unsafe { ffi!(FPDFAction_GetURIPath(doc, action, std::ptr::null_mut(), 0)) } as usize;
    if needed < 2 {
        return None;
    }
    let mut buf: Vec<u8> = vec![0; needed];
    let written = unsafe {
        ffi!(FPDFAction_GetURIPath(
            doc,
            action,
            buf.as_mut_ptr() as *mut std::os::raw::c_void,
            needed as std::os::raw::c_ulong,
        ))
    } as usize;
    if written < 2 {
        return None;
    }
    // `written` includes the trailing NUL.
    let end = written.saturating_sub(1).min(buf.len());
    let uri = String::from_utf8_lossy(&buf[..end])
        .trim_matches(char::from(0))
        .to_string();
    if uri.is_empty() { None } else { Some(uri) }
}

const FS_IDENTITY: pdfium_sys::FS_MATRIX = pdfium_sys::FS_MATRIX {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

/// Compose two affine matrices: `result(p) = outer(inner(p))`.
fn compose_matrix(
    outer: &pdfium_sys::FS_MATRIX,
    inner: &pdfium_sys::FS_MATRIX,
) -> pdfium_sys::FS_MATRIX {
    pdfium_sys::FS_MATRIX {
        a: outer.a * inner.a + outer.c * inner.b,
        b: outer.b * inner.a + outer.d * inner.b,
        c: outer.a * inner.c + outer.c * inner.d,
        d: outer.b * inner.c + outer.d * inner.d,
        e: outer.a * inner.e + outer.c * inner.f + outer.e,
        f: outer.b * inner.e + outer.d * inner.f + outer.f,
    }
}

/// Recursively collect path objects, descending into Form XObjects. `parent`
/// is the accumulated form matrix mapping this object's content space into
/// page space (identity at the top level).
fn collect_path_objects(
    obj: pdfium_sys::FPDF_PAGEOBJECT,
    parent: &pdfium_sys::FS_MATRIX,
    vp: &ViewportTransform,
    depth: usize,
    out: &mut Vec<PathObject>,
) {
    const MAX_FORM_DEPTH: usize = 6;
    let obj_type = unsafe { ffi!(FPDFPageObj_GetType(obj)) };

    if obj_type == pdfium_sys::FPDF_PAGEOBJ_FORM as i32 {
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        let mut fm = FS_IDENTITY;
        unsafe { ffi!(FPDFPageObj_GetMatrix(obj, &mut fm)) };
        let combined = compose_matrix(parent, &fm);
        let n = unsafe { ffi!(FPDFFormObj_CountObjects(obj)) };
        for i in 0..n {
            let child = unsafe { ffi!(FPDFFormObj_GetObject(obj, i as std::os::raw::c_ulong)) };
            if child.is_null() {
                continue;
            }
            collect_path_objects(child, &combined, vp, depth + 1, out);
        }
        return;
    }

    if obj_type != pdfium_sys::FPDF_PAGEOBJ_PATH as i32 {
        return;
    }

    // Object → content-space matrix, composed with the accumulated form
    // matrix to reach page space.
    let mut m = FS_IDENTITY;
    unsafe { ffi!(FPDFPageObj_GetMatrix(obj, &mut m)) };
    let m = compose_matrix(parent, &m);

    // GetBounds reports bounds in the object's content-stream space (its own
    // matrix applied, ancestor form matrices not). Lift the corners through
    // the parent matrix, then to viewport.
    let mut left = 0.0f32;
    let mut bottom = 0.0f32;
    let mut right = 0.0f32;
    let mut top = 0.0f32;
    let ok = unsafe {
        ffi!(FPDFPageObj_GetBounds(
            obj,
            &mut left,
            &mut bottom,
            &mut right,
            &mut top
        ))
    };
    if ok == 0 {
        return;
    }
    let corners = [(left, bottom), (left, top), (right, bottom), (right, top)];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        let px = parent.a * x + parent.c * y + parent.e;
        let py = parent.b * x + parent.d * y + parent.f;
        min_x = min_x.min(px);
        max_x = max_x.max(px);
        min_y = min_y.min(py);
        max_y = max_y.max(py);
    }
    let bbox = vp.transform_bounds(&RectF {
        left: min_x,
        top: max_y,
        right: max_x,
        bottom: min_y,
    });

    // Draw mode → is_filled / is_stroked.
    let mut fill_mode = 0i32;
    let mut stroke_bool = 0i32;
    let dm_ok = unsafe { ffi!(FPDFPath_GetDrawMode(obj, &mut fill_mode, &mut stroke_bool)) };
    let (is_filled, is_stroked) = if dm_ok != 0 {
        (
            fill_mode != pdfium_sys::FPDF_FILLMODE_NONE as i32,
            stroke_bool != 0,
        )
    } else {
        (false, false)
    };

    // Colors are reported as RGBA channels in 0..=255 cuint.
    let stroke_color =
        read_color(|r, g, b, a| unsafe { ffi!(FPDFPageObj_GetStrokeColor(obj, r, g, b, a)) });
    let fill_color =
        read_color(|r, g, b, a| unsafe { ffi!(FPDFPageObj_GetFillColor(obj, r, g, b, a)) });

    let mut stroke_width = 0.0f32;
    unsafe { ffi!(FPDFPageObj_GetStrokeWidth(obj, &mut stroke_width)) };

    // Walk segments. Points are in the object's local coords; apply the
    // composed matrix → page, then viewport transform.
    let n_segs = unsafe { ffi!(FPDFPath_CountSegments(obj)) };
    let mut segments = Vec::with_capacity(n_segs.max(0) as usize);
    for si in 0..n_segs {
        let seg = unsafe { ffi!(FPDFPath_GetPathSegment(obj, si)) };
        if seg.is_null() {
            continue;
        }
        let mut sx = 0.0f32;
        let mut sy = 0.0f32;
        let pt_ok = unsafe { ffi!(FPDFPathSegment_GetPoint(seg, &mut sx, &mut sy)) };
        if pt_ok == 0 {
            continue;
        }
        let ty = unsafe { ffi!(FPDFPathSegment_GetType(seg)) };
        let close = unsafe { ffi!(FPDFPathSegment_GetClose(seg)) } != 0;
        let kind = match ty as u32 {
            pdfium_sys::FPDF_SEGMENT_MOVETO => SegmentKind::MoveTo,
            pdfium_sys::FPDF_SEGMENT_LINETO => SegmentKind::LineTo,
            pdfium_sys::FPDF_SEGMENT_BEZIERTO => SegmentKind::BezierTo,
            _ => continue,
        };

        // Apply the composed matrix (FS_MATRIX is column-major a/b/c/d/e/f
        // matching the PDF text-matrix convention used elsewhere).
        let page_x = m.a * sx + m.c * sy + m.e;
        let page_y = m.b * sx + m.d * sy + m.f;
        let (x, y) = vp.transform_point(page_x, page_y);
        segments.push(PathSegment { kind, x, y, close });
    }

    out.push(PathObject {
        bbox,
        stroke_color,
        fill_color,
        stroke_width,
        is_stroked,
        is_filled,
        segments,
    });
}

/// Helper: call a PDFium getter for RGBA color channels and pack into our `Color`.
/// Returns None when the FFI call reports failure.
fn read_color<F>(getter: F) -> Option<Color>
where
    F: FnOnce(*mut u32, *mut u32, *mut u32, *mut u32) -> i32,
{
    let mut r = 0u32;
    let mut g = 0u32;
    let mut b = 0u32;
    let mut a = 0u32;
    let ok = getter(&mut r, &mut g, &mut b, &mut a);
    if ok == 0 {
        return None;
    }
    Some(Color {
        r: r as u8,
        g: g as u8,
        b: b as u8,
        a: a as u8,
    })
}

/// Recursion limit for nested form XObjects in `filled_path_bounds`.
const MAX_FORM_DEPTH: u32 = 4;

/// Compose two FS_MATRIX transforms: the result applies `inner` first,
/// then `outer` (i.e. `outer ∘ inner`).
fn compose_matrices(
    outer: &pdfium_sys::FS_MATRIX,
    inner: &pdfium_sys::FS_MATRIX,
) -> pdfium_sys::FS_MATRIX {
    pdfium_sys::FS_MATRIX {
        a: outer.a * inner.a + outer.c * inner.b,
        b: outer.b * inner.a + outer.d * inner.b,
        c: outer.a * inner.c + outer.c * inner.d,
        d: outer.b * inner.c + outer.d * inner.d,
        e: outer.a * inner.e + outer.c * inner.f + outer.e,
        f: outer.b * inner.e + outer.d * inner.f + outer.f,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_filled_paths(
    obj: pdfium_sys::FPDF_PAGEOBJECT,
    transform: Option<&pdfium_sys::FS_MATRIX>,
    page_width: f32,
    page_height: f32,
    min_size_pt: f32,
    max_page_coverage: f32,
    depth: u32,
    out: &mut Vec<ImageBounds>,
) {
    let obj_type = unsafe { ffi!(FPDFPageObj_GetType(obj)) };

    if obj_type == pdfium_sys::FPDF_PAGEOBJ_FORM as i32 {
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        // Child bounds are reported in the form's coordinate space, so the
        // form matrix (composed with any outer form transforms) must be
        // applied to map them into page space.
        let mut m = pdfium_sys::FS_MATRIX {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        let has_m = unsafe { ffi!(FPDFPageObj_GetMatrix(obj, &mut m)) } != 0;
        let combined = match (transform, has_m) {
            (Some(outer), true) => Some(compose_matrices(outer, &m)),
            (Some(outer), false) => Some(*outer),
            (None, true) => Some(m),
            (None, false) => None,
        };

        let child_count = unsafe { ffi!(FPDFFormObj_CountObjects(obj)) };
        for i in 0..child_count {
            let child = unsafe { ffi!(FPDFFormObj_GetObject(obj, i as std::os::raw::c_ulong)) };
            if child.is_null() {
                continue;
            }
            collect_filled_paths(
                child,
                combined.as_ref(),
                page_width,
                page_height,
                min_size_pt,
                max_page_coverage,
                depth + 1,
                out,
            );
        }
        return;
    }

    if obj_type != pdfium_sys::FPDF_PAGEOBJ_PATH as i32 {
        return;
    }

    // Only filled paths can be glyph outlines; skip stroke-only paths
    // (table borders, rules, underlines).
    let mut fill_mode: std::os::raw::c_int = 0;
    let mut stroke: pdfium_sys::FPDF_BOOL = 0;
    let ok = unsafe { ffi!(FPDFPath_GetDrawMode(obj, &mut fill_mode, &mut stroke)) };
    if ok == 0 || fill_mode == pdfium_sys::FPDF_FILLMODE_NONE as i32 {
        return;
    }

    // Skip light or transparent fills: glyph outlines are drawn in ink-like
    // (dark, opaque) colors, while table zebra striping and section shading
    // use light pastels. Light-on-dark text still gets caught because the
    // dark background rect itself is a dark filled path. Paths whose fill
    // color can't be read (pattern/shading fills) are kept conservatively.
    let mut r: std::os::raw::c_uint = 0;
    let mut g: std::os::raw::c_uint = 0;
    let mut b: std::os::raw::c_uint = 0;
    let mut a: std::os::raw::c_uint = 0;
    let ok = unsafe {
        ffi!(FPDFPageObj_GetFillColor(
            obj, &mut r, &mut g, &mut b, &mut a
        ))
    };
    if ok != 0 {
        if a < 128 {
            return;
        }
        let luminance = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
        if luminance > 140.0 {
            return;
        }
    }

    let mut left: f32 = 0.0;
    let mut bottom: f32 = 0.0;
    let mut right: f32 = 0.0;
    let mut top: f32 = 0.0;
    let ok = unsafe {
        ffi!(FPDFPageObj_GetBounds(
            obj,
            &mut left,
            &mut bottom,
            &mut right,
            &mut top
        ))
    };
    if ok == 0 {
        return;
    }

    if let Some(m) = transform {
        let corners = [(left, bottom), (right, bottom), (left, top), (right, top)];
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for (x, y) in corners {
            let tx = m.a * x + m.c * y + m.e;
            let ty = m.b * x + m.d * y + m.f;
            min_x = min_x.min(tx);
            max_x = max_x.max(tx);
            min_y = min_y.min(ty);
            max_y = max_y.max(ty);
        }
        left = min_x;
        right = max_x;
        bottom = min_y;
        top = max_y;
    }

    let w = right - left;
    let h = top - bottom;

    if w < min_size_pt || h < min_size_pt {
        return;
    }
    if w > page_width * max_page_coverage && h > page_height * max_page_coverage {
        return;
    }

    out.push(ImageBounds {
        x: left,
        y: page_height - top,
        width: w,
        height: h,
    });
}

/// Pre-computed affine transform from PDF page space to viewport space.
/// Avoids repeated FFI calls to `FPDF_PageToDevice` by probing 3 points
/// once and deriving the 6 affine coefficients.
#[derive(Debug, Clone, Copy)]
pub struct ViewportTransform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl ViewportTransform {
    /// Transform a single point from page space to viewport space.
    #[inline]
    pub fn transform_point(&self, page_x: f32, page_y: f32) -> (f32, f32) {
        (
            self.a * page_x + self.b * page_y + self.e,
            self.c * page_x + self.d * page_y + self.f,
        )
    }

    /// Transform a bounding rect from page space to viewport space.
    #[inline]
    pub fn transform_bounds(&self, page_bounds: &RectF) -> RectF {
        let (ll_x, ll_y) = self.transform_point(page_bounds.left, page_bounds.bottom);
        let (ur_x, ur_y) = self.transform_point(page_bounds.right, page_bounds.top);
        RectF {
            left: ll_x.min(ur_x),
            top: ll_y.min(ur_y),
            right: ll_x.max(ur_x),
            bottom: ll_y.max(ur_y),
        }
    }
}

impl<'doc, 'lib: 'doc> Page<'doc, 'lib> {
    /// Build a `ViewportTransform` by probing 3 points through PDFium.
    /// This makes 3 FFI calls total, after which all transforms are pure math.
    pub fn viewport_transform(&self, view_box: &RectF) -> ViewportTransform {
        let (e, f) = self.page_to_viewport(view_box, 0.0, 0.0);
        let (ax_e, cx_f) = self.page_to_viewport(view_box, 1.0, 0.0);
        let (by_e, dy_f) = self.page_to_viewport(view_box, 0.0, 1.0);

        ViewportTransform {
            a: ax_e - e,
            b: by_e - e,
            c: cx_f - f,
            d: dy_f - f,
            e,
            f,
        }
    }
}

impl Drop for Page<'_, '_> {
    fn drop(&mut self) {
        unsafe { ffi!(FPDF_ClosePage(self.handle)) };
    }
}
