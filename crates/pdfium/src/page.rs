use std::marker::PhantomData;

use crate::bitmap::Bitmap;
use crate::document::{Document, FormEnvironment};
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

/// Metadata for an embedded image placement retained by the extraction
/// filters. Top-level images keep their existing page-object order; images
/// nested inside Form XObjects follow in depth-first order.
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

const MAX_IMAGE_FORM_DEPTH: usize = 8;

#[derive(Clone, Copy)]
struct NestedImageObject {
    object: pdfium_sys::FPDF_PAGEOBJECT,
    parent_matrix: pdfium_sys::FS_MATRIX,
    object_index: usize,
}

struct PageImageObjects {
    images: Vec<NestedImageObject>,
    error_count: u32,
}

/// Collect image placements depth-first within a Form XObject. PDF producers
/// commonly wrap an entire page in a form, so a top-level-only
/// `FPDFPage_CountObjects` walk misses every image on the page.
fn collect_image_objects(
    object: pdfium_sys::FPDF_PAGEOBJECT,
    parent_matrix: &pdfium_sys::FS_MATRIX,
    depth: usize,
    next_image_index: &mut usize,
    out: &mut Vec<NestedImageObject>,
    error_count: &mut u32,
) {
    let object_type = unsafe { ffi!(FPDFPageObj_GetType(object)) };
    if object_type == pdfium_sys::FPDF_PAGEOBJ_FORM as i32 {
        if depth >= MAX_IMAGE_FORM_DEPTH {
            *error_count = error_count.saturating_add(1);
            return;
        }
        let mut form_matrix = FS_IDENTITY;
        unsafe { ffi!(FPDFPageObj_GetMatrix(object, &mut form_matrix)) };
        let combined = compose_matrix(parent_matrix, &form_matrix);
        let child_count = unsafe { ffi!(FPDFFormObj_CountObjects(object)) };
        for index in 0..child_count {
            let child = unsafe {
                ffi!(FPDFFormObj_GetObject(
                    object,
                    index as std::os::raw::c_ulong
                ))
            };
            if !child.is_null() {
                collect_image_objects(
                    child,
                    &combined,
                    depth + 1,
                    next_image_index,
                    out,
                    error_count,
                );
            }
        }
        return;
    }

    if object_type == pdfium_sys::FPDF_PAGEOBJ_IMAGE as i32 {
        out.push(NestedImageObject {
            object,
            parent_matrix: *parent_matrix,
            object_index: *next_image_index,
        });
        *next_image_index += 1;
    }
}

fn page_image_objects(page: pdfium_sys::FPDF_PAGE) -> PageImageObjects {
    let object_count = unsafe { ffi!(FPDFPage_CountObjects(page)) };
    let mut next_image_index = 0;
    let mut images = Vec::new();
    let mut forms = Vec::new();
    let mut error_count = 0;

    // Preserve the historical object indices of top-level images. Nested
    // placements are appended in a second pass, so discovering a form does not
    // renumber a later top-level image that callers may already reference.
    for index in 0..object_count {
        let object = unsafe { ffi!(FPDFPage_GetObject(page, index)) };
        if object.is_null() {
            continue;
        }
        match unsafe { ffi!(FPDFPageObj_GetType(object)) } as u32 {
            pdfium_sys::FPDF_PAGEOBJ_IMAGE => {
                images.push(NestedImageObject {
                    object,
                    parent_matrix: FS_IDENTITY,
                    object_index: next_image_index,
                });
                next_image_index += 1;
            }
            pdfium_sys::FPDF_PAGEOBJ_FORM => forms.push(object),
            _ => {}
        }
    }

    for form in forms {
        collect_image_objects(
            form,
            &FS_IDENTITY,
            0,
            &mut next_image_index,
            &mut images,
            &mut error_count,
        );
    }
    PageImageObjects {
        images,
        error_count,
    }
}

/// `FPDFPageObj_GetBounds` includes the object's own matrix but not ancestor
/// Form matrices. Transform all four corners by the accumulated parent matrix
/// so rotated or mirrored forms still produce a correct page-space AABB.
fn nested_object_bounds(
    object: pdfium_sys::FPDF_PAGEOBJECT,
    parent: &pdfium_sys::FS_MATRIX,
) -> Option<RectF> {
    let mut left = 0.0;
    let mut bottom = 0.0;
    let mut right = 0.0;
    let mut top = 0.0;
    let ok = unsafe {
        ffi!(FPDFPageObj_GetBounds(
            object,
            &mut left,
            &mut bottom,
            &mut right,
            &mut top
        ))
    };
    if ok == 0 {
        return None;
    }

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in [(left, bottom), (left, top), (right, bottom), (right, top)] {
        let transformed_x = parent.a * x + parent.c * y + parent.e;
        let transformed_y = parent.b * x + parent.d * y + parent.f;
        min_x = min_x.min(transformed_x);
        min_y = min_y.min(transformed_y);
        max_x = max_x.max(transformed_x);
        max_y = max_y.max(transformed_y);
    }
    Some(RectF {
        left: min_x,
        top: max_y,
        right: max_x,
        bottom: min_y,
    })
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

/// One PDF annotation with geometry normalized to viewport space (top-left
/// origin, 72 DPI). String fields mirror the standard annotation dictionary.
#[derive(Debug, Clone)]
pub struct PdfAnnotation {
    pub object_number: Option<i32>,
    pub subtype: String,
    pub contents: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub title: Option<String>,
    pub rect: Option<RectF>,
    pub quadpoint_rects: Vec<RectF>,
    pub uri: Option<String>,
}

/// One AcroForm widget with its resolved field metadata. A logical radio or
/// checkbox field may appear more than once when it owns several widgets.
#[derive(Debug, Clone)]
pub struct PdfFormField {
    pub id: String,
    pub field_type: String,
    pub page: u32,
    pub annotation_index: i32,
    pub widget_index: i32,
    pub object_number: Option<i32>,
    pub name: Option<String>,
    pub alternate_name: Option<String>,
    pub value: Option<String>,
    pub export_value: Option<String>,
    pub field_flags: i32,
    pub control_count: Option<i32>,
    pub control_index: Option<i32>,
    pub checked: Option<bool>,
    pub rect: Option<RectF>,
    pub options: Vec<String>,
    pub selected_options: Vec<String>,
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
        self.render_with_form(dpi, None)
    }

    /// Render the page to a BGRA bitmap, drawing form-field appearances
    /// (filled values, checkbox states) on top via `FPDF_FFLDraw` when a form
    /// environment is supplied. Without it, PDFium only paints widget
    /// annotations' static appearance streams, so filled-in form data can be
    /// missing from the raster. The page open/close form notifications are
    /// wrapped around the render, mirroring the LlamaParse extract binary.
    pub fn render_with_form(
        &self,
        dpi: f32,
        form: Option<&FormEnvironment>,
    ) -> Result<Bitmap<'lib>, PdfiumError> {
        if let Some(form) = form {
            unsafe {
                ffi!(FORM_OnAfterLoadPage(self.handle, form.handle));
                ffi!(FORM_DoPageAAction(
                    self.handle,
                    form.handle,
                    pdfium_sys::FPDFPAGE_AACTION_OPEN as i32,
                ));
            }
        }
        let result = self.render_bitmap(dpi, form);
        if let Some(form) = form {
            unsafe {
                ffi!(FORM_DoPageAAction(
                    self.handle,
                    form.handle,
                    pdfium_sys::FPDFPAGE_AACTION_CLOSE as i32,
                ));
                ffi!(FORM_OnBeforeClosePage(self.handle, form.handle));
            }
        }
        result
    }

    fn render_bitmap(
        &self,
        dpi: f32,
        form: Option<&FormEnvironment>,
    ) -> Result<Bitmap<'lib>, PdfiumError> {
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

        if let Some(form) = form {
            // NOTE: like the LlamaParse extract binary, popup annotations are
            // not drawn (flags 0 for the form layer draw).
            unsafe {
                ffi!(FPDF_FFLDraw(
                    form.handle,
                    bitmap.handle(),
                    self.handle,
                    0,
                    0,
                    width,
                    height,
                    0,
                    0,
                ));
            }
        }

        Ok(bitmap)
    }

    /// Union of all top-level page object bounds, in PDF page space.
    /// `None` when the page has no content objects. Mirrors the LlamaParse
    /// extract binary's `Parse_getContentBounds`.
    pub fn content_bounds(&self) -> Option<RectF> {
        let count = unsafe { ffi!(FPDFPage_CountObjects(self.handle)) };
        let mut bounds: Option<RectF> = None;
        for i in 0..count {
            let obj = unsafe { ffi!(FPDFPage_GetObject(self.handle, i)) };
            if obj.is_null() {
                continue;
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
                continue;
            }
            bounds = Some(match bounds {
                None => RectF {
                    left,
                    top,
                    right,
                    bottom,
                },
                Some(prev) => RectF {
                    left: prev.left.min(left),
                    top: prev.top.max(top),
                    right: prev.right.max(right),
                    bottom: prev.bottom.min(bottom),
                },
            });
        }
        bounds
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
        let page_images = page_image_objects(self.handle);
        let mut results = Vec::new();
        let mut error_count = page_images.error_count;
        for nested in page_images.images {
            let obj = nested.object;
            let Some(page_bounds) = nested_object_bounds(obj, &nested.parent_matrix) else {
                error_count += 1;
                continue;
            };

            let w = page_bounds.right - page_bounds.left;
            let h = page_bounds.top - page_bounds.bottom;

            if w < min_size_pt || h < min_size_pt {
                continue;
            }
            if w > page_width * max_page_coverage && h > page_height * max_page_coverage {
                continue;
            }

            let viewport = self.bounds_to_viewport(&view_box, &page_bounds);

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
                    let page_matrix = compose_matrix(&nested.parent_matrix, &matrix);
                    page_matrix
                        .b
                        .atan2(page_matrix.a)
                        .to_degrees()
                        .rem_euclid(360.0)
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
                object_index: nested.object_index,
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
    /// Top-level images keep their page-object order. Images nested inside Form
    /// XObjects follow in depth-first order.
    pub fn render_image_object(&self, image_obj_index: usize) -> Result<Bitmap<'lib>, PdfiumError> {
        for image in page_image_objects(self.handle).images {
            if image.object_index == image_obj_index {
                let bmp_handle = unsafe {
                    ffi!(FPDFImageObj_GetRenderedBitmap(
                        self.doc_handle,
                        self.handle,
                        image.object
                    ))
                };
                if bmp_handle.is_null() {
                    return Err(PdfiumError::OperationFailed);
                }
                // Wrap in our Bitmap (which will call Destroy on drop)
                return Ok(unsafe { Bitmap::from_handle(bmp_handle) });
            }
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

    /// Enumerate all page annotations. Unlike [`Page::links`], this preserves
    /// non-link subtypes and annotation dictionary metadata for public output.
    pub fn annotations(&self, view_box: &RectF) -> Vec<PdfAnnotation> {
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        let mut out = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                continue;
            }

            let subtype = unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) };
            let mut rect = pdfium_sys::FS_RECTF::default();
            let rect = if unsafe { ffi!(FPDFAnnot_GetRect(annot, &mut rect)) } != 0 {
                Some(self.bounds_to_viewport(
                    view_box,
                    &RectF {
                        left: rect.left,
                        top: rect.top,
                        right: rect.right,
                        bottom: rect.bottom,
                    },
                ))
            } else {
                None
            };

            let mut quadpoint_rects = Vec::new();
            if unsafe { ffi!(FPDFAnnot_HasAttachmentPoints(annot)) } != 0 {
                let quad_count = unsafe { ffi!(FPDFAnnot_CountAttachmentPoints(annot)) };
                quadpoint_rects.reserve(quad_count);
                for quad_index in 0..quad_count {
                    let mut quad = pdfium_sys::FS_QUADPOINTSF::default();
                    if unsafe { ffi!(FPDFAnnot_GetAttachmentPoints(annot, quad_index, &mut quad)) }
                        == 0
                    {
                        continue;
                    }
                    let bounds = RectF {
                        left: quad.x1.min(quad.x2).min(quad.x3).min(quad.x4),
                        bottom: quad.y1.min(quad.y2).min(quad.y3).min(quad.y4),
                        right: quad.x1.max(quad.x2).max(quad.x3).max(quad.x4),
                        top: quad.y1.max(quad.y2).max(quad.y3).max(quad.y4),
                    };
                    quadpoint_rects.push(self.bounds_to_viewport(view_box, &bounds));
                }
            }

            let uri = if subtype == pdfium_sys::FPDF_ANNOT_LINK as i32 {
                let link = unsafe { ffi!(FPDFAnnot_GetLink(annot)) };
                if link.is_null() {
                    None
                } else {
                    let action = unsafe { ffi!(FPDFLink_GetAction(link)) };
                    if action.is_null() {
                        None
                    } else {
                        read_uri_path(self.doc_handle, action)
                    }
                }
            } else {
                None
            };

            out.push(PdfAnnotation {
                object_number: match unsafe { ffi!(FPDFAnnot_GetObjNum(annot)) } {
                    number if number > 0 => Some(number),
                    _ => None,
                },
                subtype: annotation_subtype_name(subtype).to_string(),
                contents: read_annotation_string(annot, b"Contents\0"),
                created: read_annotation_string(annot, b"CreationDate\0"),
                modified: read_annotation_string(annot, b"M\0"),
                title: read_annotation_string(annot, b"T\0"),
                rect,
                quadpoint_rects,
                uri,
            });
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
        }
        out
    }

    /// Whether any visible annotation paints text through its `/AP /N`
    /// appearance stream.
    ///
    /// PDFium's text API tokenizes the page content stream only — text drawn
    /// by an annotation appearance is rendered but never extracted. Pages
    /// authored that way (a common anti-copy / production-tool pattern) look
    /// identical to a blank page through [`Page::text_items`], so callers need
    /// this to tell "nothing here" from "content OCR can recover".
    ///
    /// Stops at the first text object found; hidden and popup annotations are
    /// skipped because they are not painted in the render OCR would see.
    pub fn has_annotation_text(&self) -> bool {
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        for index in 0..count {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                continue;
            }
            let subtype = unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) };
            let flags = unsafe { ffi!(FPDFAnnot_GetFlags(annot)) };
            let hidden = flags & pdfium_sys::FPDF_ANNOT_FLAG_HIDDEN as i32 != 0;
            let found = !hidden
                && subtype != pdfium_sys::FPDF_ANNOT_POPUP as i32
                && annotation_paints_text_shallow(annot);
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
            if found {
                return true;
            }
        }
        false
    }

    /// Viewport rects of the visible AcroForm widgets that paint text through
    /// their appearance streams. Empty when the page has no such widget, which
    /// is the signal not to flatten.
    ///
    /// PDFium's page text API omits these glyphs until the page is flattened,
    /// so the rects double as the only regions where flattening can introduce
    /// text — callers use them to scope duplicate detection instead of
    /// rescanning the whole page.
    pub fn form_widget_text_rects(&self, view_box: &RectF) -> Vec<RectF> {
        let mut rects = Vec::new();
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        for index in 0..count {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                continue;
            }
            if unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) } == pdfium_sys::FPDF_ANNOT_WIDGET as i32
                && annotation_paints_text_deep(annot)
            {
                let mut rect = pdfium_sys::FS_RECTF::default();
                if unsafe { ffi!(FPDFAnnot_GetRect(annot, &mut rect)) } != 0 {
                    rects.push(self.bounds_to_viewport(
                        view_box,
                        &RectF {
                            left: rect.left,
                            top: rect.top,
                            right: rect.right,
                            bottom: rect.bottom,
                        },
                    ));
                }
            }
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
        }
        rects
    }

    /// Whether any text object already in the page content stream overlaps one
    /// of `rects`.
    ///
    /// Flattening replaces the page content under a widget's rect with that
    /// widget's appearance, so text already drawn there is lost. This is the
    /// cheap probe for that situation: it walks page-object bounding boxes
    /// only — no text page, no glyph decoding — so the common form page (whose
    /// widget rects sit over blank space) pays a bounds walk instead of a
    /// second full text extraction.
    pub fn text_objects_overlap(&self, view_box: &RectF, rects: &[RectF]) -> bool {
        if rects.is_empty() {
            return false;
        }
        let count = unsafe { ffi!(FPDFPage_CountObjects(self.handle)) };
        for i in 0..count {
            let obj = unsafe { ffi!(FPDFPage_GetObject(self.handle, i)) };
            if obj.is_null()
                || unsafe { ffi!(FPDFPageObj_GetType(obj)) } != pdfium_sys::FPDF_PAGEOBJ_TEXT as i32
            {
                continue;
            }
            let (mut left, mut bottom, mut right, mut top) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            if unsafe {
                ffi!(FPDFPageObj_GetBounds(
                    obj,
                    &mut left,
                    &mut bottom,
                    &mut right,
                    &mut top
                ))
            } == 0
            {
                continue;
            }
            let bounds = self.bounds_to_viewport(
                view_box,
                &RectF {
                    left,
                    top,
                    right,
                    bottom,
                },
            );
            if rects.iter().any(|rect| {
                bounds.left < rect.right
                    && bounds.right > rect.left
                    && bounds.top < rect.bottom
                    && bounds.bottom > rect.top
            }) {
                return true;
            }
        }
        false
    }

    /// Promote visible form-widget appearances into page content without
    /// admitting comment, markup, stamp, or other annotation appearances into
    /// the text layer.
    ///
    /// PDFium's flatten operation is page-wide and otherwise consumes every
    /// visible annotation. Hide non-widget annotations in this disposable
    /// extraction document first; callers snapshot annotation metadata and
    /// reopen the pristine input for any later rendering work.
    ///
    /// Returns true only when PDFium changed the page. Returns false — leaving
    /// extraction on the original page content — when the pdfium build omits
    /// the flatten API, or when suppression or flattening fails, in which case
    /// any changed flags are restored first.
    pub fn flatten_form_widgets_for_display(&self) -> bool {
        // `fpdf_flatten.h` is an optional pdfium API; trimmed builds omit it.
        // Missing it costs form-value text, not the whole parse.
        let Some(api) = FlattenApi::load() else {
            return false;
        };
        let mut suppressed = Vec::new();
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        for index in 0..count {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                continue;
            }
            let subtype = unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) };
            if subtype != pdfium_sys::FPDF_ANNOT_WIDGET as i32 {
                let flags = unsafe { ffi!(FPDFAnnot_GetFlags(annot)) };
                if flags & pdfium_sys::FPDF_ANNOT_FLAG_HIDDEN as i32 == 0 {
                    let hidden_flags = flags | pdfium_sys::FPDF_ANNOT_FLAG_HIDDEN as i32;
                    let changed = unsafe { (api.set_flags)(annot, hidden_flags) } != 0;
                    unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
                    if !changed {
                        restore_annotation_flags(&api, self.handle, &suppressed);
                        return false;
                    }
                    suppressed.push((index, flags));
                    continue;
                }
            }
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
        }

        let result = unsafe { (api.flatten)(self.handle, pdfium_sys::FLAT_NORMALDISPLAY as i32) };
        if result != pdfium_sys::FLATTEN_SUCCESS as i32 {
            restore_annotation_flags(&api, self.handle, &suppressed);
        }
        result == pdfium_sys::FLATTEN_SUCCESS as i32
    }

    /// Enumerate AcroForm widget annotations and resolve their field values
    /// through PDFium's form-fill environment.
    pub fn form_fields(
        &self,
        form: &FormEnvironment<'_, '_>,
        view_box: &RectF,
        page_number: u32,
    ) -> Vec<PdfFormField> {
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        let mut out = Vec::new();
        let mut widget_index = 0;
        for annotation_index in 0..count {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, annotation_index)) };
            if annot.is_null() {
                continue;
            }
            if unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) } != pdfium_sys::FPDF_ANNOT_WIDGET as i32
            {
                unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
                continue;
            }

            let parent =
                unsafe { ffi!(FPDFAnnot_GetLinkedAnnot(annot, b"Parent\0".as_ptr().cast())) };
            let name = first_present([
                read_form_string(
                    form.handle,
                    annot,
                    |handle, annotation, buffer, len| unsafe {
                        ffi!(FPDFAnnot_GetFormFieldName(handle, annotation, buffer, len))
                    },
                ),
                read_annotation_string(annot, b"T\0"),
                (!parent.is_null())
                    .then(|| read_annotation_string(parent, b"T\0"))
                    .flatten(),
            ]);
            let alternate_name = first_present([
                read_form_string(
                    form.handle,
                    annot,
                    |handle, annotation, buffer, len| unsafe {
                        ffi!(FPDFAnnot_GetFormFieldAlternateName(
                            handle, annotation, buffer, len
                        ))
                    },
                ),
                read_annotation_string(annot, b"TU\0"),
                (!parent.is_null())
                    .then(|| read_annotation_string(parent, b"TU\0"))
                    .flatten(),
            ]);
            let value = first_present([
                read_form_string(
                    form.handle,
                    annot,
                    |handle, annotation, buffer, len| unsafe {
                        ffi!(FPDFAnnot_GetFormFieldValue(handle, annotation, buffer, len))
                    },
                ),
                read_annotation_string(annot, b"V\0"),
                (!parent.is_null())
                    .then(|| read_annotation_string(parent, b"V\0"))
                    .flatten(),
            ]);
            let appearance_state = read_annotation_string(annot, b"AS\0");
            let mut export_value = first_present([
                read_form_string(
                    form.handle,
                    annot,
                    |handle, annotation, buffer, len| unsafe {
                        ffi!(FPDFAnnot_GetFormFieldExportValue(
                            handle, annotation, buffer, len
                        ))
                    },
                ),
                read_annotation_string(annot, b"V\0"),
            ]);
            if export_value.is_none() {
                export_value = appearance_state
                    .as_ref()
                    .filter(|state| state.as_str() != "Off")
                    .cloned();
            }

            let raw_type = unsafe { ffi!(FPDFAnnot_GetFormFieldType(form.handle, annot)) };
            let object_number = match unsafe { ffi!(FPDFAnnot_GetObjNum(annot)) } {
                number if number > 0 => Some(number),
                _ => None,
            };
            let id = name
                .clone()
                .or_else(|| object_number.map(|n| format!("pdf-object-{n}")))
                .unwrap_or_else(|| format!("page-{page_number}-widget-{widget_index}"));
            let rect = annotation_rect(self, annot, view_box);
            let option_count = unsafe { ffi!(FPDFAnnot_GetOptionCount(form.handle, annot)) };
            let mut options = Vec::new();
            let mut selected_options = Vec::new();
            for option_index in 0..option_count.max(0) {
                if let Some(label) = read_form_option_label(form.handle, annot, option_index) {
                    if unsafe { ffi!(FPDFAnnot_IsOptionSelected(form.handle, annot, option_index)) }
                        != 0
                    {
                        selected_options.push(label.clone());
                    }
                    options.push(label);
                }
            }
            let is_checkable = raw_type == pdfium_sys::FPDF_FORMFIELD_CHECKBOX as i32
                || raw_type == pdfium_sys::FPDF_FORMFIELD_RADIOBUTTON as i32;
            let checked = is_checkable.then(|| {
                (unsafe { ffi!(FPDFAnnot_IsChecked(form.handle, annot)) }) != 0
                    || appearance_state
                        .as_ref()
                        .is_some_and(|state| state != "Off")
            });
            let control_count = is_checkable
                .then(|| unsafe { ffi!(FPDFAnnot_GetFormControlCount(form.handle, annot)) })
                .filter(|value| *value >= 0);
            let control_index = is_checkable
                .then(|| unsafe { ffi!(FPDFAnnot_GetFormControlIndex(form.handle, annot)) })
                .filter(|value| *value >= 0);

            out.push(PdfFormField {
                id,
                field_type: form_field_type_name(raw_type).to_owned(),
                page: page_number,
                annotation_index,
                widget_index,
                object_number,
                name,
                alternate_name,
                value,
                export_value,
                field_flags: unsafe { ffi!(FPDFAnnot_GetFormFieldFlags(form.handle, annot)) },
                control_count,
                control_index,
                checked,
                rect,
                options,
                selected_options,
            });
            if !parent.is_null() {
                unsafe { ffi!(FPDFPage_CloseAnnot(parent)) };
            }
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
            widget_index += 1;
        }
        out
    }
}

/// The optional page-flatten API, resolved together so a build missing either
/// half degrades to "no flattening" rather than failing the whole pdfium load.
struct FlattenApi {
    flatten:
        unsafe extern "C" fn(pdfium_sys::FPDF_PAGE, std::os::raw::c_int) -> std::os::raw::c_int,
    set_flags: unsafe extern "C" fn(
        pdfium_sys::FPDF_ANNOTATION,
        std::os::raw::c_int,
    ) -> pdfium_sys::FPDF_BOOL,
}

impl FlattenApi {
    #[cfg(not(target_arch = "wasm32"))]
    fn load() -> Option<Self> {
        let bindings = pdfium_sys::dynamic::pdfium();
        Some(Self {
            flatten: bindings.FPDFPage_Flatten?,
            set_flags: bindings.FPDFAnnot_SetFlags?,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn load() -> Option<Self> {
        Some(Self {
            flatten: pdfium_sys::FPDFPage_Flatten,
            set_flags: pdfium_sys::FPDFAnnot_SetFlags,
        })
    }
}

/// Whether the annotation's appearance paints text at its top level.
///
/// Deliberately shallow and HIDDEN-agnostic: this backs the long-standing
/// `AnnotationText` complexity signal, and widening it would silently reroute
/// pages to OCR. [`annotation_paints_text_deep`] is the form-widget variant.
fn annotation_paints_text_shallow(annot: pdfium_sys::FPDF_ANNOTATION) -> bool {
    let object_count = unsafe { ffi!(FPDFAnnot_GetObjectCount(annot)) };
    (0..object_count).any(|object_index| {
        let object = unsafe { ffi!(FPDFAnnot_GetObject(annot, object_index)) };
        !object.is_null()
            && unsafe { ffi!(FPDFPageObj_GetType(object)) } == pdfium_sys::FPDF_PAGEOBJ_TEXT as i32
    })
}

/// Whether a widget's appearance paints text, descending into nested form
/// XObjects.
///
/// PDFium parses an `/AP /N` stream into top-level objects, so a producer that
/// wraps variable text in `/Tx BMC ... /Fm0 Do EMC` (Acrobat and several
/// server-side fillers do) yields a form object, not a text object. Without the
/// descent those filled fields look empty and never get flattened.
fn annotation_paints_text_deep(annot: pdfium_sys::FPDF_ANNOTATION) -> bool {
    // Invisible/hidden/noview widgets are not painted, so flattening them would
    // introduce text the reader never sees.
    let flags = unsafe { ffi!(FPDFAnnot_GetFlags(annot)) };
    let suppressed = pdfium_sys::FPDF_ANNOT_FLAG_INVISIBLE
        | pdfium_sys::FPDF_ANNOT_FLAG_HIDDEN
        | pdfium_sys::FPDF_ANNOT_FLAG_NOVIEW;
    if flags & suppressed as i32 != 0 {
        return false;
    }

    let object_count = unsafe { ffi!(FPDFAnnot_GetObjectCount(annot)) };
    (0..object_count).any(|object_index| {
        let object = unsafe { ffi!(FPDFAnnot_GetObject(annot, object_index)) };
        !object.is_null() && object_paints_text(object, 0)
    })
}

/// Depth-bounded search for a text object, following form XObjects.
fn object_paints_text(object: pdfium_sys::FPDF_PAGEOBJECT, depth: u32) -> bool {
    // Appearance nesting is shallow in practice; the cap only guards against
    // pathological or cyclic documents.
    const MAX_DEPTH: u32 = 8;
    match unsafe { ffi!(FPDFPageObj_GetType(object)) } as u32 {
        pdfium_sys::FPDF_PAGEOBJ_TEXT => true,
        pdfium_sys::FPDF_PAGEOBJ_FORM if depth < MAX_DEPTH => {
            let count = unsafe { ffi!(FPDFFormObj_CountObjects(object)) };
            (0..count).any(|index| {
                let child = unsafe {
                    ffi!(FPDFFormObj_GetObject(
                        object,
                        index as std::os::raw::c_ulong
                    ))
                };
                !child.is_null() && object_paints_text(child, depth + 1)
            })
        }
        _ => false,
    }
}

fn restore_annotation_flags(
    api: &FlattenApi,
    page: pdfium_sys::FPDF_PAGE,
    originals: &[(i32, i32)],
) {
    for &(index, flags) in originals {
        let annot = unsafe { ffi!(FPDFPage_GetAnnot(page, index)) };
        if annot.is_null() {
            continue;
        }
        unsafe {
            (api.set_flags)(annot, flags);
            ffi!(FPDFPage_CloseAnnot(annot));
        }
    }
}

fn form_field_type_name(field_type: i32) -> &'static str {
    match field_type as u32 {
        pdfium_sys::FPDF_FORMFIELD_PUSHBUTTON => "pushbutton",
        pdfium_sys::FPDF_FORMFIELD_CHECKBOX => "checkbox",
        pdfium_sys::FPDF_FORMFIELD_RADIOBUTTON => "radio",
        pdfium_sys::FPDF_FORMFIELD_COMBOBOX => "combobox",
        pdfium_sys::FPDF_FORMFIELD_LISTBOX => "listbox",
        pdfium_sys::FPDF_FORMFIELD_TEXTFIELD => "text",
        pdfium_sys::FPDF_FORMFIELD_SIGNATURE => "signature",
        _ => "unknown",
    }
}

fn first_present<const N: usize>(values: [Option<String>; N]) -> Option<String> {
    values.into_iter().flatten().find(|value| !value.is_empty())
}

fn read_form_string(
    form: pdfium_sys::FPDF_FORMHANDLE,
    annot: pdfium_sys::FPDF_ANNOTATION,
    mut getter: impl FnMut(
        pdfium_sys::FPDF_FORMHANDLE,
        pdfium_sys::FPDF_ANNOTATION,
        *mut u16,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
) -> Option<String> {
    let needed = getter(form, annot, std::ptr::null_mut(), 0) as usize;
    if needed < 2 {
        return None;
    }
    let mut buffer = vec![0u16; needed.div_ceil(2)];
    let written = getter(
        form,
        annot,
        buffer.as_mut_ptr(),
        needed as std::os::raw::c_ulong,
    ) as usize;
    if written < 2 {
        return None;
    }
    let units = (written / 2).min(buffer.len());
    let end = units.saturating_sub(usize::from(buffer.get(units.saturating_sub(1)) == Some(&0)));
    let value = String::from_utf16_lossy(&buffer[..end]);
    (!value.is_empty()).then_some(value)
}

fn read_form_option_label(
    form: pdfium_sys::FPDF_FORMHANDLE,
    annot: pdfium_sys::FPDF_ANNOTATION,
    index: i32,
) -> Option<String> {
    read_form_string(form, annot, |handle, annotation, buffer, len| unsafe {
        ffi!(FPDFAnnot_GetOptionLabel(
            handle, annotation, index, buffer, len
        ))
    })
}

fn annotation_rect(
    page: &Page<'_, '_>,
    annot: pdfium_sys::FPDF_ANNOTATION,
    view_box: &RectF,
) -> Option<RectF> {
    let mut rect = pdfium_sys::FS_RECTF::default();
    (unsafe { ffi!(FPDFAnnot_GetRect(annot, &mut rect)) } != 0).then(|| {
        page.bounds_to_viewport(
            view_box,
            &RectF {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
        )
    })
}

fn annotation_subtype_name(subtype: pdfium_sys::FPDF_ANNOTATION_SUBTYPE) -> &'static str {
    const NAMES: [&str; 29] = [
        "unknown",
        "text",
        "link",
        "freetext",
        "line",
        "square",
        "circle",
        "polygon",
        "polyline",
        "highlight",
        "underline",
        "squiggly",
        "strikeout",
        "stamp",
        "caret",
        "ink",
        "popup",
        "fileattachment",
        "sound",
        "movie",
        "widget",
        "screen",
        "printermark",
        "trapnet",
        "watermark",
        "threed",
        "richmedia",
        "xfawidget",
        "redact",
    ];
    usize::try_from(subtype)
        .ok()
        .and_then(|index| NAMES.get(index))
        .copied()
        .unwrap_or("unknown")
}

fn read_annotation_string(
    annot: pdfium_sys::FPDF_ANNOTATION,
    key: &'static [u8],
) -> Option<String> {
    let key = key.as_ptr().cast();
    let needed = unsafe {
        ffi!(FPDFAnnot_GetStringValue(
            annot,
            key,
            std::ptr::null_mut(),
            0
        ))
    } as usize;
    if needed < 2 {
        return None;
    }
    let mut buf = vec![0u16; needed.div_ceil(2)];
    let written = unsafe {
        ffi!(FPDFAnnot_GetStringValue(
            annot,
            key,
            buf.as_mut_ptr(),
            needed as std::os::raw::c_ulong
        ))
    } as usize;
    if written < 2 {
        return None;
    }
    let units = (written / 2).min(buf.len());
    let end = if buf.get(units.saturating_sub(1)) == Some(&0) {
        units.saturating_sub(1)
    } else {
        units
    };
    let value = String::from_utf16_lossy(&buf[..end]);
    if value.is_empty() { None } else { Some(value) }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Library;

    #[test]
    fn nested_form_matrix_composition_transforms_path_points_in_order() {
        // Outer form translates by (100, 20), inner form scales by (2, 3),
        // and the path object translates by (4, 5). The collector composes
        // these in outer(inner(path(point))) order.
        let outer = pdfium_sys::FS_MATRIX {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 100.0,
            f: 20.0,
        };
        let inner = pdfium_sys::FS_MATRIX {
            a: 2.0,
            b: 0.0,
            c: 0.0,
            d: 3.0,
            e: 0.0,
            f: 0.0,
        };
        let object = pdfium_sys::FS_MATRIX {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 4.0,
            f: 5.0,
        };
        let nested = compose_matrix(&compose_matrix(&outer, &inner), &object);
        let x = nested.a * 1.0 + nested.c * 2.0 + nested.e;
        let y = nested.b * 1.0 + nested.d * 2.0 + nested.f;
        assert_eq!((x, y), (110.0, 41.0));

        let viewport = ViewportTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: 0.0,
            f: 200.0,
        };
        assert_eq!(viewport.transform_point(x, y), (110.0, 159.0));
    }

    fn annotation_pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R 5 0 R] >>",
            "<< /Type /Annot /Subtype /Highlight /Rect [10 20 100 40] /QuadPoints [10 40 100 40 10 20 100 20] /Contents (review this) /T (Reviewer) /CreationDate (D:20260102030405Z) /M (D:20260103040506Z) >>",
            "<< /Type /Annot /Subtype /Link /Rect [10 50 100 70] /Border [0 0 0] /A << /S /URI /URI (https://example.com) >> >>",
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
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

    fn nested_form_image_pdf() -> Vec<u8> {
        let form_stream = b"q 100 0 0 80 20 30 cm /Im0 Do Q";
        // Paint the form before a direct image. Discovery must still preserve
        // the historical index (0) of the top-level image and append the
        // newly visible nested placement after it.
        let page_stream = b"q /Fm0 Do Q q 30 0 0 30 150 150 cm /Im1 Do Q";
        let objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
                 /Resources << /XObject << /Fm0 4 0 R /Im1 6 0 R >> >> /Contents 5 0 R >>"
            )
            .into_bytes(),
            [
                format!(
                    "<< /Type /XObject /Subtype /Form /BBox [0 0 200 200] \
                     /Matrix [1 0 0 1 10 20] \
                     /Resources << /XObject << /Im0 6 0 R >> >> \
                     /Length {} >>\nstream\n",
                    form_stream.len()
                )
                .into_bytes(),
                form_stream.to_vec(),
                b"\nendstream".to_vec(),
            ]
            .concat(),
            [
                format!("<< /Length {} >>\nstream\n", page_stream.len()).into_bytes(),
                page_stream.to_vec(),
                b"\nendstream".to_vec(),
            ]
            .concat(),
            b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 \
              /ColorSpace /DeviceRGB /BitsPerComponent 8 \
              /Filter /ASCIIHexDecode /Length 7 >>\nstream\nFF0000>\nendstream"
                .to_vec(),
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
    fn image_objects_descend_into_form_xobjects() {
        let bytes = nested_form_image_pdf();
        let library = Library::init();
        let document = library.load_document_from_bytes(&bytes, None).unwrap();
        let page = document.page(0).unwrap();

        let images = page.image_objects(25.0, 0.9, false);

        assert_eq!(images.error_count, 0);
        assert_eq!(images.images.len(), 2);
        assert_eq!(images.images[0].object_index, 0);
        let direct_bounds = images.images[0].bounds;
        assert!((direct_bounds.x - 150.0).abs() < 0.1);
        assert!((direct_bounds.y - 20.0).abs() < 0.1);
        assert!((direct_bounds.width - 30.0).abs() < 0.1);
        assert!((direct_bounds.height - 30.0).abs() < 0.1);

        assert_eq!(images.images[1].object_index, 1);
        let bounds = images.images[1].bounds;
        assert!((bounds.x - 30.0).abs() < 0.1, "x={}", bounds.x);
        assert!((bounds.y - 70.0).abs() < 0.1, "y={}", bounds.y);
        assert!((bounds.width - 100.0).abs() < 0.1, "width={}", bounds.width);
        assert!(
            (bounds.height - 80.0).abs() < 0.1,
            "height={}",
            bounds.height
        );

        let with_data = page.image_objects(25.0, 0.9, true);
        assert_eq!(with_data.images.len(), 2);
        assert_eq!(with_data.images[1].pixel_width, 1);
        assert_eq!(with_data.images[1].pixel_height, 1);
        let bitmap = page
            .render_image_object(with_data.images[1].object_index)
            .expect("nested image placement should remain renderable by index");
        assert!(bitmap.width() > 0);
        assert!(bitmap.height() > 0);
    }

    #[test]
    fn extracts_annotation_metadata_geometry_and_link_uri() {
        let bytes = annotation_pdf();
        let library = Library::init();
        let document = library.load_document_from_bytes(&bytes, None).unwrap();
        let page = document.page(0).unwrap();
        let view_box = page.view_box().unwrap();
        let annotations = page.annotations(&view_box);

        assert_eq!(annotations.len(), 2);
        let highlight = &annotations[0];
        assert_eq!(highlight.subtype, "highlight");
        assert_eq!(highlight.contents.as_deref(), Some("review this"));
        assert_eq!(highlight.title.as_deref(), Some("Reviewer"));
        assert_eq!(highlight.created.as_deref(), Some("D:20260102030405Z"));
        assert_eq!(highlight.modified.as_deref(), Some("D:20260103040506Z"));
        let rect = highlight.rect.as_ref().unwrap();
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (10.0, 160.0, 100.0, 180.0)
        );
        assert_eq!(highlight.quadpoint_rects.len(), 1);

        let link = &annotations[1];
        assert_eq!(link.subtype, "link");
        assert_eq!(link.uri.as_deref(), Some("https://example.com"));
        assert_eq!(link.rect.as_ref().unwrap().top, 130.0);
    }
}
