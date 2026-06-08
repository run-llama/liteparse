use std::marker::PhantomData;

use crate::bitmap::Bitmap;
use crate::document::Document;
use crate::error::PdfiumError;
use crate::ffi;
use crate::text_page::TextPage;
use crate::types::RectF;

/// Bounding box of an embedded image object on a page.
/// Coordinates are in PDF points with top-left origin (Y-down).
#[derive(Debug, Clone, Copy)]
pub struct ImageBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Embedded image object metadata, including the image-object index needed
/// to render the image via [`Page::render_image_object`].
#[derive(Debug, Clone, Copy)]
pub struct ImageObjectInfo {
    pub image_obj_index: usize,
    pub bounds: ImageBounds,
}

pub struct Page<'doc> {
    pub(crate) handle: pdfium_sys::FPDF_PAGE,
    pub(crate) doc_handle: pdfium_sys::FPDF_DOCUMENT,
    pub(crate) _doc: PhantomData<&'doc Document>,
}

impl Page<'_> {
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

    pub fn text(&self) -> Result<TextPage<'_>, PdfiumError> {
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
    pub fn render(&self, dpi: f32) -> Result<Bitmap, PdfiumError> {
        let scale = dpi / 72.0;
        let width = (self.width() * scale).round() as i32;
        let height = (self.height() * scale).round() as i32;

        let bitmap = Bitmap::new(width, height)?;

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
        self.image_objects(min_size_pt, max_page_coverage)
            .into_iter()
            .map(|info| info.bounds)
            .collect()
    }

    /// Extract embedded image object metadata on this page.
    ///
    /// The returned `image_obj_index` is the index among image objects only,
    /// matching the index expected by [`Page::render_image_object`].
    pub fn image_objects(&self, min_size_pt: f32, max_page_coverage: f32) -> Vec<ImageObjectInfo> {
        let page_width = self.width();
        let page_height = self.height();
        let obj_count = unsafe { ffi!(FPDFPage_CountObjects(self.handle)) };
        let mut results = Vec::new();
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
                image_idx += 1;
                continue;
            }

            let w = right - left;
            let h = top - bottom;

            if w >= min_size_pt
                && h >= min_size_pt
                && !(w > page_width * max_page_coverage && h > page_height * max_page_coverage)
            {
                // Convert from PDF coords (bottom-left origin) to viewport (top-left origin)
                results.push(ImageObjectInfo {
                    image_obj_index: image_idx,
                    bounds: ImageBounds {
                        x: left,
                        y: page_height - top,
                        width: w,
                        height: h,
                    },
                });
            }
            image_idx += 1;
        }

        results
    }

    /// Get the rendered bitmap of a specific embedded image object by index.
    /// The index corresponds to the order from iterating page objects (image objects only).
    pub fn render_image_object(&self, image_obj_index: usize) -> Result<Bitmap, PdfiumError> {
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

impl Page<'_> {
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

impl Drop for Page<'_> {
    fn drop(&mut self) {
        unsafe { ffi!(FPDF_ClosePage(self.handle)) };
    }
}
