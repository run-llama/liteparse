use std::marker::PhantomData;

use crate::error::PdfiumError;
use crate::ffi;
use crate::library::Library;

/// A BGRA pixel buffer owned by PDFium.
///
/// The `'lib` lifetime ties the bitmap to a held [`Library`] lock, so it
/// cannot be created (or destroyed via `Drop`) outside the PDFium critical
/// section.
pub struct Bitmap<'lib> {
    handle: pdfium_sys::FPDF_BITMAP,
    _lib: PhantomData<&'lib Library>,
}

impl<'lib> Bitmap<'lib> {
    /// Wrap an existing FPDF_BITMAP handle (takes ownership, will destroy on drop).
    ///
    /// # Safety
    /// The handle must be a valid, non-null bitmap that the caller owns,
    /// and the caller must hold a [`Library`] for at least `'lib`.
    pub unsafe fn from_handle(handle: pdfium_sys::FPDF_BITMAP) -> Self {
        Bitmap {
            handle,
            _lib: PhantomData,
        }
    }

    /// Create a new BGRA bitmap with the given dimensions.
    ///
    /// # Safety
    /// The caller must hold a [`Library`] for at least `'lib` (PDFium FFI is
    /// not thread-safe). `'lib` is not constrained by an argument, so callers
    /// must ensure it cannot outlive the held lock — usually by inferring it
    /// from the call site (e.g. returning a `Bitmap<'lib>` from a method on
    /// `Page<'_, 'lib>`, whose existence already proves the lock is held).
    pub unsafe fn new(width: i32, height: i32) -> Result<Self, PdfiumError> {
        let handle = unsafe {
            ffi!(FPDFBitmap_CreateEx(
                width,
                height,
                pdfium_sys::FPDFBitmap_BGRA as i32,
                std::ptr::null_mut(),
                0, // stride=0 lets pdfium choose
            ))
        };
        if handle.is_null() {
            return Err(PdfiumError::OperationFailed);
        }
        Ok(Bitmap {
            handle,
            _lib: PhantomData,
        })
    }

    /// Copy `rows` scanlines from `src` — starting at its row `src_y` — into
    /// `self` starting at row `dst_y`. Both bitmaps must be BGRA and the same
    /// width; strides may differ.
    ///
    /// Used to assemble a strip-rendered page: each strip is rendered slightly
    /// taller than it owns (a one-row halo top and bottom) so that every owned
    /// row is interior to the render and free of clip-edge anti-aliasing, then
    /// only the owned rows are copied into the full bitmap — leaving no seam.
    pub fn blit_rows_from(&self, src: &Bitmap<'_>, src_y: i32, dst_y: i32, rows: i32) {
        let row_bytes = (self.width() * 4) as usize;
        let dst_stride = self.stride() as usize;
        let src_stride = src.stride() as usize;
        let dst = unsafe { ffi!(FPDFBitmap_GetBuffer(self.handle)) } as *mut u8;
        let src_ptr = unsafe { ffi!(FPDFBitmap_GetBuffer(src.handle)) } as *const u8;
        for r in 0..rows as usize {
            let so = (src_y as usize + r) * src_stride;
            let doff = (dst_y as usize + r) * dst_stride;
            // SAFETY: caller guarantees the row ranges lie within both buffers
            // and the width matches; source and destination never overlap
            // (distinct bitmaps).
            unsafe {
                std::ptr::copy_nonoverlapping(src_ptr.add(so), dst.add(doff), row_bytes);
            }
        }
    }

    pub fn handle(&self) -> pdfium_sys::FPDF_BITMAP {
        self.handle
    }

    pub fn width(&self) -> i32 {
        unsafe { ffi!(FPDFBitmap_GetWidth(self.handle)) }
    }

    pub fn height(&self) -> i32 {
        unsafe { ffi!(FPDFBitmap_GetHeight(self.handle)) }
    }

    pub fn stride(&self) -> i32 {
        unsafe { ffi!(FPDFBitmap_GetStride(self.handle)) }
    }

    /// Fill a rectangle with an ARGB color (0xAARRGGBB).
    pub fn fill_rect(&self, left: i32, top: i32, width: i32, height: i32, color: u64) {
        unsafe {
            ffi!(FPDFBitmap_FillRect(
                self.handle,
                left,
                top,
                width,
                height,
                // necessary for windows -> expected `u32`, found `u64`
                #[allow(clippy::useless_conversion)]
                color.try_into().unwrap(),
            ));
        }
    }

    /// Get the raw pixel buffer as a byte slice.
    /// Format is BGRA, row-major, with `stride()` bytes per row.
    pub fn buffer(&self) -> &[u8] {
        let ptr = unsafe { ffi!(FPDFBitmap_GetBuffer(self.handle)) };
        let len = (self.stride() * self.height()) as usize;
        unsafe { std::slice::from_raw_parts(ptr as *const u8, len) }
    }

    /// Convert the BGRA buffer to RGBA in a new Vec.
    pub fn to_rgba(&self) -> Vec<u8> {
        let width = self.width() as usize;
        let height = self.height() as usize;
        let stride = self.stride() as usize;
        let src = self.buffer();
        let mut rgba = Vec::with_capacity(width * height * 4);

        for y in 0..height {
            let row = &src[y * stride..y * stride + width * 4];
            for pixel in row.chunks_exact(4) {
                // BGRA -> RGBA
                rgba.push(pixel[2]); // R
                rgba.push(pixel[1]); // G
                rgba.push(pixel[0]); // B
                rgba.push(pixel[3]); // A
            }
        }

        rgba
    }

    /// Convert the BGRA buffer to tightly-packed RGB in a new Vec, dropping the
    /// alpha channel (pages render onto opaque white, so alpha is constant 255).
    pub fn to_rgb(&self) -> Vec<u8> {
        let width = self.width() as usize;
        let height = self.height() as usize;
        let stride = self.stride() as usize;
        let src = self.buffer();
        let mut rgb = Vec::with_capacity(width * height * 3);

        for y in 0..height {
            let row = &src[y * stride..y * stride + width * 4];
            for pixel in row.chunks_exact(4) {
                // BGRA -> RGB (drop A)
                rgb.push(pixel[2]); // R
                rgb.push(pixel[1]); // G
                rgb.push(pixel[0]); // B
            }
        }

        rgb
    }

    /// Convert the BGRA buffer to tightly-packed 8-bit grayscale (1 byte/px)
    /// using Rec. 601 luma weights.
    pub fn to_luma(&self) -> Vec<u8> {
        let width = self.width() as usize;
        let height = self.height() as usize;
        let stride = self.stride() as usize;
        let src = self.buffer();
        let mut luma = Vec::with_capacity(width * height);

        for y in 0..height {
            let row = &src[y * stride..y * stride + width * 4];
            for pixel in row.chunks_exact(4) {
                let (b, g, r) = (pixel[0] as u32, pixel[1] as u32, pixel[2] as u32);
                luma.push(((77 * r + 150 * g + 29 * b) >> 8) as u8);
            }
        }

        luma
    }
}

impl Drop for Bitmap<'_> {
    fn drop(&mut self) {
        unsafe { ffi!(FPDFBitmap_Destroy(self.handle)) };
    }
}
