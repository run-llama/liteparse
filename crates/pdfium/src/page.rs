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

pub(crate) fn image_object_data(
    obj: pdfium_sys::FPDF_PAGEOBJECT,
    decoded: bool,
) -> Option<Vec<u8>> {
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

/// One PDF annotation with geometry normalized to viewport space (top-left
/// origin, 72 DPI). String fields mirror the standard annotation dictionary:
/// `None` when the key is absent, `Some("")` when it is present but empty, so
/// callers can tell the two apart.
#[derive(Debug, Clone)]
pub struct PdfAnnotation {
    pub object_number: Option<i32>,
    pub subtype: String,
    pub contents: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub title: Option<String>,
    pub rect: Option<RectF>,
    /// Whether pdfium reports attachment points for this annotation
    /// (`FPDFAnnot_HasAttachmentPoints`: a subtype test — links and the
    /// text-markup subtypes). Tells "declares no quads" from "subtype never has
    /// them" when `quadpoint_rects` is empty.
    pub has_attachment_points: bool,
    /// Bounding boxes of the `/QuadPoints` quads, when `has_attachment_points`.
    pub quadpoint_rects: Vec<RectF>,
    /// The link's URI, decoded lossily and only when non-empty.
    pub uri: Option<String>,
    /// The link's URI action bytes exactly as stored (no trailing NUL), present
    /// whenever the link annotation carries a URI action — including an empty
    /// URI or one that is not valid UTF-8, which `uri` cannot represent.
    pub uri_raw: Option<Vec<u8>>,
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
    /// `/UserUnit` multiplier (1.0 for normal pages). PDFium reports all
    /// geometry in raw MediaBox units, so viewport-space output and render
    /// pixel sizing multiply by this to recover the page's real scale (see
    /// `crate::user_unit`).
    pub(crate) user_unit: f32,
    pub(crate) _doc: PhantomData<&'doc Document<'lib>>,
}

impl<'doc, 'lib: 'doc> Page<'doc, 'lib> {
    /// Page width in raw MediaBox units, exactly as PDFium reports it
    /// (ignores `/UserUnit`; multiply by [`Self::user_unit`] for the real
    /// size, or use [`Self::viewport_size`] which already does).
    pub fn width(&self) -> f32 {
        unsafe { ffi!(FPDF_GetPageWidthF(self.handle)) }
    }

    /// Page height in raw MediaBox units (see [`Self::width`]).
    pub fn height(&self) -> f32 {
        unsafe { ffi!(FPDF_GetPageHeightF(self.handle)) }
    }

    /// The page's `/UserUnit` multiplier, 1.0 when absent.
    pub fn user_unit(&self) -> f32 {
        self.user_unit
    }

    /// The page's `/Rotate` in quarter turns clockwise (0..=3), as PDFium
    /// reports it.
    pub fn rotation(&self) -> i32 {
        unsafe { ffi!(FPDFPage_GetRotation(self.handle)) }
    }

    /// Overwrite the page's `/Rotate` with `quarter_turns` (0..=3, clockwise
    /// when displayed). PDFium writes the value into the page dictionary and
    /// recomputes the page size, so [`Self::width`] / [`Self::height`], the
    /// viewport mapping, and every later `FPDF_LoadPage` of this page in the
    /// same document see the new rotation. Values outside 0..=3 are ignored
    /// by PDFium.
    pub fn set_rotation(&self, quarter_turns: i32) {
        unsafe { ffi!(FPDFPage_SetRotation(self.handle, quarter_turns)) }
    }

    /// Replace the page's `/CropBox` (PDF user space, points, bottom-left origin).
    /// pdfium recomputes the page size, so [`Self::width`], [`Self::height`],
    /// [`Self::view_box`] and any later render see the new box. This is how a
    /// caller renders one region of a page at full resolution: crop, then render
    /// the whole (now smaller) page.
    pub fn set_crop_box(&self, left: f32, bottom: f32, right: f32, top: f32) {
        unsafe { ffi!(FPDFPage_SetCropBox(self.handle, left, bottom, right, top)) }
    }

    /// Render into a caller-owned bitmap with explicit pixel geometry and pdfium
    /// flags, then draw form fields on top when `form` is given.
    ///
    /// The page is mapped onto the `size_x × size_y` pixel rectangle whose top-left
    /// sits at (`start_x`, `start_y`) in `bitmap`; parts outside the bitmap are
    /// clipped, so a shorter bitmap with a negative `start_y` renders one horizontal
    /// strip of a tall page. The bitmap is not cleared first. For a plain DPI-based
    /// render use [`Self::render_with_form`]; this exists for consumers that need an
    /// exact edge length or a different flag set (the LlamaParse extractor renders
    /// with `FPDF_ANNOT` alone, at a pixel size it computes from a max-edge rule).
    #[allow(clippy::too_many_arguments)]
    pub fn render_into(
        &self,
        bitmap: &Bitmap<'lib>,
        start_x: i32,
        start_y: i32,
        size_x: i32,
        size_y: i32,
        form: Option<&FormEnvironment>,
        flags: i32,
    ) {
        unsafe {
            ffi!(FPDF_RenderPageBitmap(
                bitmap.handle(),
                self.handle,
                start_x,
                start_y,
                size_x,
                size_y,
                0,
                flags,
            ));
        }
        if let Some(form) = form {
            // Form layer drawn with flags 0, as the extractor does (no popups).
            unsafe {
                ffi!(FPDF_FFLDraw(
                    form.handle,
                    bitmap.handle(),
                    self.handle,
                    start_x,
                    start_y,
                    size_x,
                    size_y,
                    0,
                    0,
                ));
            }
        }
    }

    /// Page dimensions in the same rotation-adjusted viewport coordinate
    /// space returned by [`Self::page_to_viewport`]. Unlike [`Self::width`]/
    /// [`Self::height`], this applies the `/UserUnit` multiplier, so the
    /// result is in real points (1/72 inch).
    pub fn viewport_size(&self, view_box: &RectF) -> (f32, f32) {
        let mut width = (view_box.right - view_box.left).abs() * self.user_unit;
        let mut height = (view_box.top - view_box.bottom).abs() * self.user_unit;
        if matches!(self.rotation(), 1 | 3) {
            std::mem::swap(&mut width, &mut height);
        }
        (width, height)
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
        let (vw, vh) = self.viewport_size(view_box);

        // 1000x is the precision multiplier for the integer device space.
        // `/UserUnit` pages can be hundreds of thousands of points tall, so
        // cap the multiplier to keep the device box inside i32 range.
        let mult = 1000.0f32.min((i32::MAX / 4) as f32 / vw.max(vh).max(1.0));
        let device_w = (vw * mult).round() as i32;
        let device_h = (vh * mult).round() as i32;
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

        (dx as f32 / mult, dy as f32 / mult)
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
    /// environment is supplied. Without it, PDFium paints no widget
    /// annotations at all (see [`Self::unpainted_widget_indices`]). The page
    /// open/close form notifications are wrapped around the render, mirroring
    /// the LlamaParse extract binary.
    pub fn render_with_form(
        &self,
        dpi: f32,
        form: Option<&FormEnvironment>,
    ) -> Result<Bitmap<'lib>, PdfiumError> {
        let _form_page = form.map(|form| self.notify_form_page_loaded(form));
        self.render_bitmap(
            dpi,
            form,
            (pdfium_sys::FPDF_ANNOT | pdfium_sys::FPDF_PRINTING) as i32,
        )
    }

    /// Like [`Self::render_with_form`], but the page as a viewer shows it on
    /// screen rather than as printed: annotations without the Print flag are
    /// drawn and Hidden/NoView ones are not, which is what Poppler and MuPDF
    /// render too.
    pub fn render_on_screen(
        &self,
        dpi: f32,
        form: Option<&FormEnvironment>,
    ) -> Result<Bitmap<'lib>, PdfiumError> {
        let _form_page = form.map(|form| self.notify_form_page_loaded(form));
        self.render_bitmap(dpi, form, pdfium_sys::FPDF_ANNOT as i32)
    }

    /// Tell the form-fill environment this page is open: `FORM_OnAfterLoadPage`
    /// then the page's `/AA` open action. The returned guard runs the close
    /// action and `FORM_OnBeforeClosePage` when dropped, so drop it before the
    /// page itself goes away.
    ///
    /// This is not only for rendering. Loading the page into the environment is
    /// when pdfium builds its widget objects and regenerates appearance streams
    /// that are missing or stale (`NeedAppearances` forms), and a checkbox's
    /// export value is read off that appearance dictionary — so form-field
    /// extraction that skips the notification sees fewer values than a viewer
    /// would.
    pub fn notify_form_page_loaded<'a>(&'a self, form: &'a FormEnvironment) -> FormPageGuard<'a> {
        unsafe {
            ffi!(FORM_OnAfterLoadPage(self.handle, form.handle));
            ffi!(FORM_DoPageAAction(
                self.handle,
                form.handle,
                pdfium_sys::FPDFPAGE_AACTION_OPEN as i32,
            ));
        }
        FormPageGuard {
            page: self.handle,
            form: form.handle,
            _lifetime: std::marker::PhantomData,
        }
    }

    /// Load this page into `form` without running any page action. Loading is
    /// when PDFium builds its widget objects and writes an appearance stream for
    /// every widget that lacks a valid one — checkboxes, buttons and text
    /// fields saved without `/AP` — so a later render or overlay copy can draw
    /// them. The returned guard unloads the page (`FORM_OnBeforeClosePage`);
    /// drop it before the page.
    pub fn load_form_widgets<'a>(&'a self, form: &'a FormEnvironment) -> FormWidgetsGuard<'a> {
        unsafe { ffi!(FORM_OnAfterLoadPage(self.handle, form.handle)) };
        FormWidgetsGuard {
            page: self.handle,
            form: form.handle,
            _lifetime: std::marker::PhantomData,
        }
    }

    fn render_bitmap(
        &self,
        dpi: f32,
        form: Option<&FormEnvironment>,
        flags: i32,
    ) -> Result<Bitmap<'lib>, PdfiumError> {
        // `/UserUnit` scales the physical page, so honoring the requested
        // DPI means scaling the pixel size by it too — otherwise a UserUnit
        // page renders at dpi/user_unit and its text rasterizes microscopic.
        let scale = dpi / 72.0 * self.user_unit;
        let width = (self.width() * scale).round() as i32;
        let height = (self.height() * scale).round() as i32;

        // SAFETY: this method is on `Page<'_, 'lib>`, whose existence proves
        // the PDFium lock is held for `'lib`; the returned `Bitmap<'lib>` is
        // tied to that same lock lifetime.
        let bitmap = unsafe { Bitmap::new(width, height) }?;

        // Fill with white (ARGB: 0xFFFFFFFF)
        bitmap.fill_rect(0, 0, width, height, 0xFFFFFFFF);

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
            let has_attachment_points = unsafe { ffi!(FPDFAnnot_HasAttachmentPoints(annot)) } != 0;
            if has_attachment_points {
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

            let uri_raw = if subtype == pdfium_sys::FPDF_ANNOT_LINK as i32 {
                let link = unsafe { ffi!(FPDFAnnot_GetLink(annot)) };
                if link.is_null() {
                    None
                } else {
                    let action = unsafe { ffi!(FPDFLink_GetAction(link)) };
                    if action.is_null() {
                        None
                    } else {
                        read_uri_path_bytes(self.doc_handle, action)
                    }
                }
            } else {
                None
            };
            let uri = uri_raw.as_deref().and_then(decode_uri_path);

            out.push(PdfAnnotation {
                object_number: match unsafe { ffi!(FPDFAnnot_GetObjNum(annot)) } {
                    number if number > 0 => Some(number),
                    _ => None,
                },
                subtype: annotation_subtype_name(subtype).to_string(),
                contents: read_annotation_string_if_present(annot, b"Contents\0"),
                created: read_annotation_string_if_present(annot, b"CreationDate\0"),
                modified: read_annotation_string_if_present(annot, b"M\0"),
                title: read_annotation_string_if_present(annot, b"T\0"),
                rect,
                has_attachment_points,
                quadpoint_rects,
                uri,
                uri_raw,
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

    /// Whether any visible AcroForm widget on the page paints text through its
    /// appearance stream (nested form XObjects included). The cheap gate in
    /// front of [`Document::widget_appearance_copy`]: pdfium's page text API
    /// omits these glyphs until the appearances are flattened.
    pub fn has_form_widget_text(&self) -> bool {
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        (0..count).any(|index| {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                return false;
            }
            let found = unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) }
                == pdfium_sys::FPDF_ANNOT_WIDGET as i32
                && annotation_paints_text_deep(annot);
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
            found
        })
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

    /// Indices of the visible widget annotations that a render of this page
    /// would leave blank.
    ///
    /// `FPDF_RenderPageBitmap` skips widget annotations outright; `FPDF_FFLDraw`
    /// paints only the widgets the document's AcroForm field tree knows. So
    /// with no `form` every visible widget is unpainted, and with one the
    /// unpainted widgets are those PDFium cannot resolve to a form field —
    /// typically widgets orphaned from a missing or incomplete `/AcroForm`.
    ///
    /// Widgets without an appearance stream are included: loading the page
    /// into a form environment (see [`Self::load_form_widgets`]) generates the
    /// missing ones. Call this before that load, which adopts orphaned widgets
    /// into the form.
    pub fn unpainted_widget_indices(&self, form: Option<&FormEnvironment>) -> Vec<i32> {
        let mut indices = Vec::new();
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        for index in 0..count {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                continue;
            }
            let unpainted = unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) }
                == pdfium_sys::FPDF_ANNOT_WIDGET as i32
                && unsafe { ffi!(FPDFAnnot_GetFlags(annot)) } & SCREEN_HIDDEN == 0
                && form.is_none_or(
                    |form| unsafe { ffi!(FPDFAnnot_GetFormFieldType(form.handle, annot)) } < 0,
                );
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
            if unpainted {
                indices.push(index);
            }
        }
        indices
    }

    /// Indices of the annotations a viewer paints above `widgets` (indices
    /// from [`Self::unpainted_widget_indices`]): every annotation after the
    /// first of them in `/Annots` that is shown on screen and has an
    /// appearance stream, including widgets the form layer would paint under
    /// the overlay.
    ///
    /// The normal render paints these before the widgets are drawn over it, so
    /// an opaque widget box would cover, say, an ink signature or stamp placed
    /// on top of the field. Callers lift them out of the base render with
    /// [`Self::hide_annotations`] and draw them with the widgets, so PDFium
    /// places and stacks all of them itself.
    ///
    /// Appearance streams PDFium generates (Square, Ink, Highlight and other
    /// markup saved without `/AP`) count only once written, so load the page
    /// into a form environment first (see [`Self::load_form_widgets`]).
    pub fn annotations_above(&self, widgets: &[i32]) -> Vec<i32> {
        let Some(&first_widget) = widgets.iter().min() else {
            return Vec::new();
        };
        let count = unsafe { ffi!(FPDFPage_GetAnnotCount(self.handle)) };
        (first_widget + 1..count)
            .filter(|index| !widgets.contains(index))
            .filter(|&index| {
                let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
                if annot.is_null() {
                    return false;
                }
                let shown = unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) }
                    != pdfium_sys::FPDF_ANNOT_POPUP as i32
                    && unsafe { ffi!(FPDFAnnot_GetFlags(annot)) } & SCREEN_HIDDEN == 0
                    && unsafe { ffi!(FPDFAnnot_HasKey(annot, c"AP".as_ptr())) } != 0;
                unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
                shown
            })
            .collect()
    }

    /// The annotations at `indices` that draw optional content (a layer) in
    /// their appearance, descending into nested form XObjects.
    ///
    /// Layer visibility lives in the catalog's `/OCProperties`, which a page
    /// imported into another document leaves behind, so an overlay copy would
    /// show layers that are off. Only `/OC` marked content is seen; a form
    /// XObject's own `/OC` entry is not exposed by PDFium's API. An
    /// annotation's own `/OC` entry needs no check: PDFium's render ignores it
    /// in the source document too.
    pub fn optional_content_annotations(&self, indices: &[i32]) -> Vec<i32> {
        indices
            .iter()
            .copied()
            .filter(|&index| {
                let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
                if annot.is_null() {
                    return false;
                }
                let object_count = unsafe { ffi!(FPDFAnnot_GetObjectCount(annot)) };
                let uses = (0..object_count).any(|object_index| {
                    let object = unsafe { ffi!(FPDFAnnot_GetObject(annot, object_index)) };
                    !object.is_null() && object_uses_optional_content(object, 0)
                });
                unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
                uses
            })
            .collect()
    }

    /// Whether the annotations at `a` and `b` may paint over each other: their
    /// `/Rect`s intersect, or one is `/NoRotate` on a rotated page, where it
    /// is drawn turned about its corner, outside its `/Rect`. `true` when
    /// either rect is unavailable.
    pub fn annotations_overlap(&self, a: i32, b: i32) -> bool {
        let rotated = self.rotation() != 0;
        let bounds = |index: i32| {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                return None;
            }
            let mut rect = pdfium_sys::FS_RECTF::default();
            let found = unsafe { ffi!(FPDFAnnot_GetRect(annot, &mut rect)) } != 0;
            let turned = rotated
                && unsafe { ffi!(FPDFAnnot_GetFlags(annot)) }
                    & pdfium_sys::FPDF_ANNOT_FLAG_NOROTATE as i32
                    != 0;
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
            (found && !turned).then(|| {
                (
                    rect.left.min(rect.right),
                    rect.bottom.min(rect.top),
                    rect.left.max(rect.right),
                    rect.bottom.max(rect.top),
                )
            })
        };
        match (bounds(a), bounds(b)) {
            (Some(a), Some(b)) => a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3,
            _ => true,
        }
    }

    /// Set the Hidden flag on the annotations at `indices` until the returned
    /// guard drops, which restores their original flags. `None` (nothing
    /// changed) when the pdfium build lacks `FPDFAnnot_SetFlags` or a flag
    /// cannot be set.
    pub fn hide_annotations(&self, indices: &[i32]) -> Option<HiddenAnnotations<'_>> {
        let api = FlattenApi::load()?;
        let mut originals = Vec::with_capacity(indices.len());
        for &index in indices {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(self.handle, index)) };
            if annot.is_null() {
                restore_annotation_flags(&api, self.handle, &originals);
                return None;
            }
            let flags = unsafe { ffi!(FPDFAnnot_GetFlags(annot)) };
            let changed = unsafe {
                (api.set_flags)(annot, flags | pdfium_sys::FPDF_ANNOT_FLAG_HIDDEN as i32)
            } != 0;
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
            if !changed {
                restore_annotation_flags(&api, self.handle, &originals);
                return None;
            }
            originals.push((index, flags));
        }
        Some(HiddenAnnotations {
            api,
            page: self.handle,
            originals,
            _page: PhantomData,
        })
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

/// A page's open session in a form-fill environment; see
/// [`Page::notify_form_page_loaded`]. Closing notifications run on drop.
pub struct FormPageGuard<'a> {
    page: pdfium_sys::FPDF_PAGE,
    form: pdfium_sys::FPDF_FORMHANDLE,
    _lifetime: std::marker::PhantomData<&'a ()>,
}

impl Drop for FormPageGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi!(FORM_DoPageAAction(
                self.page,
                self.form,
                pdfium_sys::FPDFPAGE_AACTION_CLOSE as i32,
            ));
            ffi!(FORM_OnBeforeClosePage(self.page, self.form));
        }
    }
}

/// Unloads a page loaded by [`Page::load_form_widgets`].
pub struct FormWidgetsGuard<'a> {
    page: pdfium_sys::FPDF_PAGE,
    form: pdfium_sys::FPDF_FORMHANDLE,
    _lifetime: std::marker::PhantomData<&'a ()>,
}

impl Drop for FormWidgetsGuard<'_> {
    fn drop(&mut self) {
        unsafe { ffi!(FORM_OnBeforeClosePage(self.page, self.form)) };
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

/// Annotation flags that keep an annotation off the screen, as PDFium's
/// render reads them. Invisible is not one: it applies only to annotation
/// types a viewer does not recognize, and PDFium paints regardless.
pub(crate) const SCREEN_HIDDEN: i32 =
    (pdfium_sys::FPDF_ANNOT_FLAG_HIDDEN | pdfium_sys::FPDF_ANNOT_FLAG_NOVIEW) as i32;

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

/// Depth-bounded search for an object inside `/OC` marked content,
/// following form XObjects.
fn object_uses_optional_content(object: pdfium_sys::FPDF_PAGEOBJECT, depth: u32) -> bool {
    const MAX_DEPTH: u32 = 8;
    let marks = unsafe { ffi!(FPDFPageObj_CountMarks(object)) };
    let marked = (0..marks).any(|index| {
        let mark = unsafe { ffi!(FPDFPageObj_GetMark(object, index as std::os::raw::c_ulong)) };
        // "OC" and its NUL: 6 bytes of UTF-16LE.
        let mut name = [0u16; 3];
        let mut needed: std::os::raw::c_ulong = 0;
        !mark.is_null()
            && unsafe {
                ffi!(FPDFPageObjMark_GetName(
                    mark,
                    name.as_mut_ptr(),
                    std::mem::size_of_val(&name) as std::os::raw::c_ulong,
                    &mut needed
                ))
            } != 0
            && needed as usize == std::mem::size_of_val(&name)
            && name == [u16::from(b'O'), u16::from(b'C'), 0]
    });
    marked
        || unsafe { ffi!(FPDFPageObj_GetType(object)) } == pdfium_sys::FPDF_PAGEOBJ_FORM as i32
            && depth < MAX_DEPTH
            && (0..unsafe { ffi!(FPDFFormObj_CountObjects(object)) }).any(|index| {
                let child = unsafe {
                    ffi!(FPDFFormObj_GetObject(
                        object,
                        index as std::os::raw::c_ulong
                    ))
                };
                !child.is_null() && object_uses_optional_content(child, depth + 1)
            })
}

/// Restores the flags [`Page::hide_annotations`] changed when dropped.
pub struct HiddenAnnotations<'page> {
    api: FlattenApi,
    page: pdfium_sys::FPDF_PAGE,
    originals: Vec<(i32, i32)>,
    _page: PhantomData<&'page ()>,
}

impl Drop for HiddenAnnotations<'_> {
    fn drop(&mut self) {
        restore_annotation_flags(&self.api, self.page, &self.originals);
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

/// One choice-field option label. An empty label is a real option (the blank
/// first entry of many combo boxes) and is kept as `Some("")`; `None` only
/// when pdfium has no label at all for the index.
fn read_form_option_label(
    form: pdfium_sys::FPDF_FORMHANDLE,
    annot: pdfium_sys::FPDF_ANNOTATION,
    index: i32,
) -> Option<String> {
    let needed = unsafe {
        ffi!(FPDFAnnot_GetOptionLabel(
            form,
            annot,
            index,
            std::ptr::null_mut(),
            0
        ))
    } as usize;
    if needed == 0 {
        return None;
    }
    Some(
        read_form_string(form, annot, |handle, annotation, buffer, len| unsafe {
            ffi!(FPDFAnnot_GetOptionLabel(
                handle, annotation, index, buffer, len
            ))
        })
        .unwrap_or_default(),
    )
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

/// An annotation dictionary string by key: `None` when the key is absent,
/// `Some("")` when present but empty. (`FPDFAnnot_GetStringValue` alone
/// reports both as an empty string, so presence is checked first.)
fn read_annotation_string_if_present(
    annot: pdfium_sys::FPDF_ANNOTATION,
    key: &'static [u8],
) -> Option<String> {
    if unsafe { ffi!(FPDFAnnot_HasKey(annot, key.as_ptr().cast())) } == 0 {
        return None;
    }
    Some(read_annotation_string(annot, key).unwrap_or_default())
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
/// The URI action's path bytes as stored, without the trailing NUL. `None`
/// when `action` is not a URI action.
fn read_uri_path_bytes(
    doc: pdfium_sys::FPDF_DOCUMENT,
    action: pdfium_sys::FPDF_ACTION,
) -> Option<Vec<u8>> {
    let needed =
        unsafe { ffi!(FPDFAction_GetURIPath(doc, action, std::ptr::null_mut(), 0)) } as usize;
    // pdfium reports `len + 1` for a URI action (the NUL), 0 for anything else.
    if needed < 1 {
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
    if written < 1 {
        return None;
    }
    buf.truncate(written.saturating_sub(1).min(buf.len()));
    Some(buf)
}

/// The lossy, NUL-trimmed, non-empty form of a URI path for callers that want
/// a string to show or match on.
fn decode_uri_path(bytes: &[u8]) -> Option<String> {
    let uri = String::from_utf8_lossy(bytes)
        .trim_matches(char::from(0))
        .to_string();
    if uri.is_empty() { None } else { Some(uri) }
}

fn read_uri_path(
    doc: pdfium_sys::FPDF_DOCUMENT,
    action: pdfium_sys::FPDF_ACTION,
) -> Option<String> {
    read_uri_path_bytes(doc, action)
        .as_deref()
        .and_then(decode_uri_path)
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
pub(crate) fn read_color<F>(getter: F) -> Option<Color>
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
