//! Runtime loading of the pdfium shared library via `libloading`.
//!
//! On non-wasm targets, pdfium is loaded at runtime instead of being linked
//! at compile time. This avoids rpath issues when `liteparse` is used as a
//! library dependency in other Rust projects.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libloading::Library;

use crate::*;

static BINDINGS: OnceLock<PdfiumBindings> = OnceLock::new();

/// The compile-time lib directory baked in by pdfium-sys's build.rs.
const PDFIUM_LIB_DIR: &str = env!("PDFIUM_LIB_DIR");

macro_rules! load_fn {
    ($lib:expr, $name:literal) => {{
        let sym = unsafe { $lib.get::<*const ()>($name.as_bytes())? };
        unsafe { std::mem::transmute(*sym) }
    }};
}

/// Holds all pdfium function pointers loaded at runtime.
pub struct PdfiumBindings {
    // Keep the library handle alive — dropping it would unload the symbols.
    _lib: Library,

    // -- Library lifecycle --
    pub FPDF_InitLibrary: unsafe extern "C" fn(),
    pub FPDF_GetLastError: unsafe extern "C" fn() -> std::os::raw::c_ulong,

    // -- Document --
    pub FPDF_LoadDocument: unsafe extern "C" fn(FPDF_STRING, FPDF_BYTESTRING) -> FPDF_DOCUMENT,
    pub FPDF_LoadMemDocument: unsafe extern "C" fn(
        *const std::os::raw::c_void,
        std::os::raw::c_int,
        FPDF_BYTESTRING,
    ) -> FPDF_DOCUMENT,
    pub FPDF_CloseDocument: unsafe extern "C" fn(FPDF_DOCUMENT),
    pub FPDF_GetPageCount: unsafe extern "C" fn(FPDF_DOCUMENT) -> std::os::raw::c_int,

    // -- Page --
    pub FPDF_LoadPage: unsafe extern "C" fn(FPDF_DOCUMENT, std::os::raw::c_int) -> FPDF_PAGE,
    pub FPDF_ClosePage: unsafe extern "C" fn(FPDF_PAGE),
    pub FPDF_GetPageWidthF: unsafe extern "C" fn(FPDF_PAGE) -> f32,
    pub FPDF_GetPageHeightF: unsafe extern "C" fn(FPDF_PAGE) -> f32,
    pub FPDF_GetPageBoundingBox: unsafe extern "C" fn(FPDF_PAGE, *mut FS_RECTF) -> FPDF_BOOL,
    pub FPDFPage_GetRotation: unsafe extern "C" fn(FPDF_PAGE) -> std::os::raw::c_int,
    pub FPDF_PageToDevice: unsafe extern "C" fn(
        FPDF_PAGE,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        f64,
        f64,
        *mut std::os::raw::c_int,
        *mut std::os::raw::c_int,
    ) -> FPDF_BOOL,

    // -- Page objects --
    pub FPDFPage_CountObjects: unsafe extern "C" fn(FPDF_PAGE) -> std::os::raw::c_int,
    pub FPDFPage_GetObject: unsafe extern "C" fn(FPDF_PAGE, std::os::raw::c_int) -> FPDF_PAGEOBJECT,
    pub FPDFPageObj_GetType: unsafe extern "C" fn(FPDF_PAGEOBJECT) -> std::os::raw::c_int,
    pub FPDFPageObj_GetBounds:
        unsafe extern "C" fn(FPDF_PAGEOBJECT, *mut f32, *mut f32, *mut f32, *mut f32) -> FPDF_BOOL,
    pub FPDFPageObj_GetMarkedContentID:
        unsafe extern "C" fn(FPDF_PAGEOBJECT) -> std::os::raw::c_int,
    pub FPDFImageObj_GetRenderedBitmap:
        unsafe extern "C" fn(FPDF_DOCUMENT, FPDF_PAGE, FPDF_PAGEOBJECT) -> FPDF_BITMAP,
    pub FPDFImageObj_GetImageDataDecoded: unsafe extern "C" fn(
        FPDF_PAGEOBJECT,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDFImageObj_GetImageDataRaw: unsafe extern "C" fn(
        FPDF_PAGEOBJECT,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDFImageObj_GetImageFilterCount:
        unsafe extern "C" fn(FPDF_PAGEOBJECT) -> std::os::raw::c_int,
    pub FPDFImageObj_GetImageFilter: unsafe extern "C" fn(
        FPDF_PAGEOBJECT,
        std::os::raw::c_int,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDFImageObj_GetImageMetadata:
        unsafe extern "C" fn(FPDF_PAGEOBJECT, FPDF_PAGE, *mut FPDF_IMAGEOBJ_METADATA) -> FPDF_BOOL,
    pub FPDFImageObj_GetImagePixelSize: unsafe extern "C" fn(
        FPDF_PAGEOBJECT,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
    ) -> FPDF_BOOL,
    pub FPDFPageObj_GetMatrix: unsafe extern "C" fn(FPDF_PAGEOBJECT, *mut FS_MATRIX) -> FPDF_BOOL,
    pub FPDFPageObj_GetStrokeColor: unsafe extern "C" fn(
        FPDF_PAGEOBJECT,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
    ) -> FPDF_BOOL,
    pub FPDFPageObj_GetFillColor: unsafe extern "C" fn(
        FPDF_PAGEOBJECT,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
    ) -> FPDF_BOOL,
    pub FPDFPageObj_GetStrokeWidth: unsafe extern "C" fn(FPDF_PAGEOBJECT, *mut f32) -> FPDF_BOOL,
    pub FPDFPath_GetDrawMode: unsafe extern "C" fn(
        FPDF_PAGEOBJECT,
        *mut std::os::raw::c_int,
        *mut FPDF_BOOL,
    ) -> FPDF_BOOL,
    pub FPDFFormObj_CountObjects: unsafe extern "C" fn(FPDF_PAGEOBJECT) -> std::os::raw::c_int,
    pub FPDFFormObj_GetObject:
        unsafe extern "C" fn(FPDF_PAGEOBJECT, std::os::raw::c_ulong) -> FPDF_PAGEOBJECT,
    pub FPDFPath_CountSegments: unsafe extern "C" fn(FPDF_PAGEOBJECT) -> std::os::raw::c_int,
    pub FPDFPath_GetPathSegment:
        unsafe extern "C" fn(FPDF_PAGEOBJECT, std::os::raw::c_int) -> FPDF_PATHSEGMENT,
    pub FPDFPathSegment_GetPoint:
        unsafe extern "C" fn(FPDF_PATHSEGMENT, *mut f32, *mut f32) -> FPDF_BOOL,
    pub FPDFPathSegment_GetType: unsafe extern "C" fn(FPDF_PATHSEGMENT) -> std::os::raw::c_int,
    pub FPDFPathSegment_GetClose: unsafe extern "C" fn(FPDF_PATHSEGMENT) -> FPDF_BOOL,

    // -- TextPage --
    pub FPDFText_LoadPage: unsafe extern "C" fn(FPDF_PAGE) -> FPDF_TEXTPAGE,
    pub FPDFText_ClosePage: unsafe extern "C" fn(FPDF_TEXTPAGE),
    pub FPDFText_CountChars: unsafe extern "C" fn(FPDF_TEXTPAGE) -> std::os::raw::c_int,
    pub FPDFText_GetUnicode:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> std::os::raw::c_uint,
    pub FPDFText_GetCharCode:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> std::os::raw::c_uint,
    pub FPDFText_GetFontSize: unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> f64,
    pub FPDFText_GetFontWeight:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> std::os::raw::c_int,
    pub FPDFText_GetFontInfo: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        std::os::raw::c_int,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
        *mut std::os::raw::c_int,
    ) -> std::os::raw::c_ulong,
    pub FPDFText_GetCharAngle: unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> f32,
    pub FPDFText_GetCharBox: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        std::os::raw::c_int,
        *mut f64,
        *mut f64,
        *mut f64,
        *mut f64,
    ) -> FPDF_BOOL,
    pub FPDFText_GetLooseCharBox:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int, *mut FS_RECTF) -> FPDF_BOOL,
    pub FPDFText_GetMatrix:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int, *mut FS_MATRIX) -> FPDF_BOOL,
    pub FPDFText_IsGenerated:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> std::os::raw::c_int,
    pub FPDFText_HasUnicodeMapError:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> std::os::raw::c_int,
    pub FPDFText_GetTextObject:
        unsafe extern "C" fn(FPDF_TEXTPAGE, std::os::raw::c_int) -> FPDF_PAGEOBJECT,
    pub FPDFText_GetStrokeColor: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        std::os::raw::c_int,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
    ) -> FPDF_BOOL,
    pub FPDFText_GetFillColor: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        std::os::raw::c_int,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
        *mut std::os::raw::c_uint,
    ) -> FPDF_BOOL,
    pub FPDFText_CountRects: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        std::os::raw::c_int,
        std::os::raw::c_int,
    ) -> std::os::raw::c_int,
    pub FPDFText_GetRect: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        std::os::raw::c_int,
        *mut f64,
        *mut f64,
        *mut f64,
        *mut f64,
    ) -> FPDF_BOOL,
    pub FPDFText_GetBoundedText: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        f64,
        f64,
        f64,
        f64,
        *mut std::os::raw::c_ushort,
        std::os::raw::c_int,
    ) -> std::os::raw::c_int,
    pub FPDFText_GetText: unsafe extern "C" fn(
        FPDF_TEXTPAGE,
        std::os::raw::c_int,
        std::os::raw::c_int,
        *mut std::os::raw::c_ushort,
    ) -> std::os::raw::c_int,
    pub FPDFTextObj_GetTextRenderMode:
        unsafe extern "C" fn(FPDF_PAGEOBJECT) -> FPDF_TEXT_RENDERMODE,

    // -- Bitmap --
    pub FPDFBitmap_CreateEx: unsafe extern "C" fn(
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        *mut std::os::raw::c_void,
        std::os::raw::c_int,
    ) -> FPDF_BITMAP,
    pub FPDFBitmap_Destroy: unsafe extern "C" fn(FPDF_BITMAP),
    pub FPDFBitmap_GetWidth: unsafe extern "C" fn(FPDF_BITMAP) -> std::os::raw::c_int,
    pub FPDFBitmap_GetHeight: unsafe extern "C" fn(FPDF_BITMAP) -> std::os::raw::c_int,
    pub FPDFBitmap_GetStride: unsafe extern "C" fn(FPDF_BITMAP) -> std::os::raw::c_int,
    pub FPDFBitmap_GetBuffer: unsafe extern "C" fn(FPDF_BITMAP) -> *mut std::os::raw::c_void,
    pub FPDFBitmap_FillRect: unsafe extern "C" fn(
        FPDF_BITMAP,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        FPDF_DWORD,
    ) -> FPDF_BOOL,

    // -- Rendering --
    pub FPDF_RenderPageBitmap: unsafe extern "C" fn(
        FPDF_BITMAP,
        FPDF_PAGE,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
        std::os::raw::c_int,
    ),

    // -- Font --
    pub FPDFTextObj_GetFont: unsafe extern "C" fn(FPDF_PAGEOBJECT) -> FPDF_FONT,
    pub FPDFFont_GetBaseFontName:
        unsafe extern "C" fn(FPDF_FONT, *mut std::os::raw::c_char, usize) -> usize,
    pub FPDFFont_GetType: unsafe extern "C" fn(FPDF_FONT) -> FPDF_FONT_TYPE,
    pub FPDFFont_GetIsEmbedded: unsafe extern "C" fn(FPDF_FONT) -> std::os::raw::c_int,
    pub FPDFFont_GetAscent: unsafe extern "C" fn(FPDF_FONT, f32, *mut f32) -> FPDF_BOOL,
    pub FPDFFont_GetDescent: unsafe extern "C" fn(FPDF_FONT, f32, *mut f32) -> FPDF_BOOL,
    pub FPDFFont_GetGlyphWidth: unsafe extern "C" fn(FPDF_FONT, u32, f32, *mut f32) -> FPDF_BOOL,
    pub FPDFFont_GetGlyphWidthFromCharCode:
        unsafe extern "C" fn(FPDF_FONT, u32, f32, *mut f32) -> FPDF_BOOL,
    pub FPDFFont_GetGlyphPathFromCharCode:
        unsafe extern "C" fn(FPDF_FONT, u32, f32) -> FPDF_GLYPHPATH,
    pub FPDFGlyphPath_CountGlyphSegments:
        unsafe extern "C" fn(FPDF_GLYPHPATH) -> std::os::raw::c_int,
    pub FPDFGlyphPath_GetGlyphPathSegment:
        unsafe extern "C" fn(FPDF_GLYPHPATH, std::os::raw::c_int) -> FPDF_PATHSEGMENT,
    pub FPDFFont_HasToUnicode: unsafe extern "C" fn(FPDF_FONT) -> FPDF_BOOL,
    pub FPDFFont_GetCharGlyphName: unsafe extern "C" fn(
        FPDF_FONT,
        u32,
        *mut std::os::raw::c_char,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDFFont_GetEncoding: unsafe extern "C" fn(
        FPDF_FONT,
        *mut std::os::raw::c_char,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDFFont_GetCharGlyphIndex: unsafe extern "C" fn(FPDF_FONT, u32) -> std::os::raw::c_int,
    pub FPDFFont_GetFontData:
        unsafe extern "C" fn(FPDF_FONT, *mut u8, usize, *mut usize) -> FPDF_BOOL,

    // -- Outline (bookmarks) --
    pub FPDFBookmark_GetFirstChild:
        unsafe extern "C" fn(FPDF_DOCUMENT, FPDF_BOOKMARK) -> FPDF_BOOKMARK,
    pub FPDFBookmark_GetNextSibling:
        unsafe extern "C" fn(FPDF_DOCUMENT, FPDF_BOOKMARK) -> FPDF_BOOKMARK,
    pub FPDFBookmark_GetTitle: unsafe extern "C" fn(
        FPDF_BOOKMARK,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDFBookmark_GetDest: unsafe extern "C" fn(FPDF_DOCUMENT, FPDF_BOOKMARK) -> FPDF_DEST,
    pub FPDFBookmark_GetAction: unsafe extern "C" fn(FPDF_BOOKMARK) -> FPDF_ACTION,
    pub FPDFAction_GetDest: unsafe extern "C" fn(FPDF_DOCUMENT, FPDF_ACTION) -> FPDF_DEST,
    pub FPDFAction_GetURIPath: unsafe extern "C" fn(
        FPDF_DOCUMENT,
        FPDF_ACTION,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDFLink_Enumerate:
        unsafe extern "C" fn(FPDF_PAGE, *mut std::os::raw::c_int, *mut FPDF_LINK) -> FPDF_BOOL,
    pub FPDFLink_GetAction: unsafe extern "C" fn(FPDF_LINK) -> FPDF_ACTION,
    pub FPDFLink_GetAnnotRect: unsafe extern "C" fn(FPDF_LINK, *mut FS_RECTF) -> FPDF_BOOL,
    pub FPDFLink_CountQuadPoints: unsafe extern "C" fn(FPDF_LINK) -> std::os::raw::c_int,
    pub FPDFLink_GetQuadPoints:
        unsafe extern "C" fn(FPDF_LINK, std::os::raw::c_int, *mut FS_QUADPOINTSF) -> FPDF_BOOL,
    pub FPDFDest_GetDestPageIndex:
        unsafe extern "C" fn(FPDF_DOCUMENT, FPDF_DEST) -> std::os::raw::c_int,
    pub FPDFDest_GetLocationInPage: unsafe extern "C" fn(
        FPDF_DEST,
        *mut FPDF_BOOL,
        *mut FPDF_BOOL,
        *mut FPDF_BOOL,
        *mut FS_FLOAT,
        *mut FS_FLOAT,
        *mut FS_FLOAT,
    ) -> FPDF_BOOL,

    // -- Structure tree --
    pub FPDF_StructTree_GetForPage: unsafe extern "C" fn(FPDF_PAGE) -> FPDF_STRUCTTREE,
    pub FPDF_StructTree_Close: unsafe extern "C" fn(FPDF_STRUCTTREE),
    pub FPDF_StructTree_CountChildren: unsafe extern "C" fn(FPDF_STRUCTTREE) -> std::os::raw::c_int,
    pub FPDF_StructTree_GetChildAtIndex:
        unsafe extern "C" fn(FPDF_STRUCTTREE, std::os::raw::c_int) -> FPDF_STRUCTELEMENT,
    pub FPDF_StructElement_GetType: unsafe extern "C" fn(
        FPDF_STRUCTELEMENT,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDF_StructElement_GetAltText: unsafe extern "C" fn(
        FPDF_STRUCTELEMENT,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDF_StructElement_GetTitle: unsafe extern "C" fn(
        FPDF_STRUCTELEMENT,
        *mut std::os::raw::c_void,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
    pub FPDF_StructElement_CountChildren:
        unsafe extern "C" fn(FPDF_STRUCTELEMENT) -> std::os::raw::c_int,
    pub FPDF_StructElement_GetChildAtIndex:
        unsafe extern "C" fn(FPDF_STRUCTELEMENT, std::os::raw::c_int) -> FPDF_STRUCTELEMENT,
    pub FPDF_StructElement_GetChildMarkedContentID:
        unsafe extern "C" fn(FPDF_STRUCTELEMENT, std::os::raw::c_int) -> std::os::raw::c_int,
    pub FPDF_StructElement_GetMarkedContentID:
        unsafe extern "C" fn(FPDF_STRUCTELEMENT) -> std::os::raw::c_int,
    pub FPDF_StructElement_GetMarkedContentIdCount:
        unsafe extern "C" fn(FPDF_STRUCTELEMENT) -> std::os::raw::c_int,
    pub FPDF_StructElement_GetMarkedContentIdAtIndex:
        unsafe extern "C" fn(FPDF_STRUCTELEMENT, std::os::raw::c_int) -> std::os::raw::c_int,
}

// SAFETY: PdfiumBindings contains only function pointers and a Library handle.
// The Library handle is thread-safe (it just holds a dlopen handle).
// Function pointers are inherently Send+Sync.
unsafe impl Send for PdfiumBindings {}
unsafe impl Sync for PdfiumBindings {}

impl PdfiumBindings {
    fn load(lib: Library) -> Result<Self, libloading::Error> {
        Ok(Self {
            FPDF_InitLibrary: load_fn!(lib, "FPDF_InitLibrary"),
            FPDF_GetLastError: load_fn!(lib, "FPDF_GetLastError"),
            FPDF_LoadDocument: load_fn!(lib, "FPDF_LoadDocument"),
            FPDF_LoadMemDocument: load_fn!(lib, "FPDF_LoadMemDocument"),
            FPDF_CloseDocument: load_fn!(lib, "FPDF_CloseDocument"),
            FPDF_GetPageCount: load_fn!(lib, "FPDF_GetPageCount"),
            FPDF_LoadPage: load_fn!(lib, "FPDF_LoadPage"),
            FPDF_ClosePage: load_fn!(lib, "FPDF_ClosePage"),
            FPDF_GetPageWidthF: load_fn!(lib, "FPDF_GetPageWidthF"),
            FPDF_GetPageHeightF: load_fn!(lib, "FPDF_GetPageHeightF"),
            FPDF_GetPageBoundingBox: load_fn!(lib, "FPDF_GetPageBoundingBox"),
            FPDFPage_GetRotation: load_fn!(lib, "FPDFPage_GetRotation"),
            FPDF_PageToDevice: load_fn!(lib, "FPDF_PageToDevice"),
            FPDFPage_CountObjects: load_fn!(lib, "FPDFPage_CountObjects"),
            FPDFPage_GetObject: load_fn!(lib, "FPDFPage_GetObject"),
            FPDFPageObj_GetType: load_fn!(lib, "FPDFPageObj_GetType"),
            FPDFPageObj_GetBounds: load_fn!(lib, "FPDFPageObj_GetBounds"),
            FPDFPageObj_GetMarkedContentID: load_fn!(lib, "FPDFPageObj_GetMarkedContentID"),
            FPDFImageObj_GetRenderedBitmap: load_fn!(lib, "FPDFImageObj_GetRenderedBitmap"),
            FPDFImageObj_GetImageDataDecoded: load_fn!(lib, "FPDFImageObj_GetImageDataDecoded"),
            FPDFImageObj_GetImageDataRaw: load_fn!(lib, "FPDFImageObj_GetImageDataRaw"),
            FPDFImageObj_GetImageFilterCount: load_fn!(lib, "FPDFImageObj_GetImageFilterCount"),
            FPDFImageObj_GetImageFilter: load_fn!(lib, "FPDFImageObj_GetImageFilter"),
            FPDFImageObj_GetImageMetadata: load_fn!(lib, "FPDFImageObj_GetImageMetadata"),
            FPDFImageObj_GetImagePixelSize: load_fn!(lib, "FPDFImageObj_GetImagePixelSize"),
            FPDFPageObj_GetMatrix: load_fn!(lib, "FPDFPageObj_GetMatrix"),
            FPDFPageObj_GetStrokeColor: load_fn!(lib, "FPDFPageObj_GetStrokeColor"),
            FPDFPageObj_GetFillColor: load_fn!(lib, "FPDFPageObj_GetFillColor"),
            FPDFPageObj_GetStrokeWidth: load_fn!(lib, "FPDFPageObj_GetStrokeWidth"),
            FPDFPath_GetDrawMode: load_fn!(lib, "FPDFPath_GetDrawMode"),
            FPDFFormObj_CountObjects: load_fn!(lib, "FPDFFormObj_CountObjects"),
            FPDFFormObj_GetObject: load_fn!(lib, "FPDFFormObj_GetObject"),
            FPDFPath_CountSegments: load_fn!(lib, "FPDFPath_CountSegments"),
            FPDFPath_GetPathSegment: load_fn!(lib, "FPDFPath_GetPathSegment"),
            FPDFPathSegment_GetPoint: load_fn!(lib, "FPDFPathSegment_GetPoint"),
            FPDFPathSegment_GetType: load_fn!(lib, "FPDFPathSegment_GetType"),
            FPDFPathSegment_GetClose: load_fn!(lib, "FPDFPathSegment_GetClose"),
            FPDFText_LoadPage: load_fn!(lib, "FPDFText_LoadPage"),
            FPDFText_ClosePage: load_fn!(lib, "FPDFText_ClosePage"),
            FPDFText_CountChars: load_fn!(lib, "FPDFText_CountChars"),
            FPDFText_GetUnicode: load_fn!(lib, "FPDFText_GetUnicode"),
            FPDFText_GetCharCode: load_fn!(lib, "FPDFText_GetCharCode"),
            FPDFText_GetFontSize: load_fn!(lib, "FPDFText_GetFontSize"),
            FPDFText_GetFontWeight: load_fn!(lib, "FPDFText_GetFontWeight"),
            FPDFText_GetFontInfo: load_fn!(lib, "FPDFText_GetFontInfo"),
            FPDFText_GetCharAngle: load_fn!(lib, "FPDFText_GetCharAngle"),
            FPDFText_GetCharBox: load_fn!(lib, "FPDFText_GetCharBox"),
            FPDFText_GetLooseCharBox: load_fn!(lib, "FPDFText_GetLooseCharBox"),
            FPDFText_GetMatrix: load_fn!(lib, "FPDFText_GetMatrix"),
            FPDFText_IsGenerated: load_fn!(lib, "FPDFText_IsGenerated"),
            FPDFText_HasUnicodeMapError: load_fn!(lib, "FPDFText_HasUnicodeMapError"),
            FPDFText_GetTextObject: load_fn!(lib, "FPDFText_GetTextObject"),
            FPDFText_GetStrokeColor: load_fn!(lib, "FPDFText_GetStrokeColor"),
            FPDFText_GetFillColor: load_fn!(lib, "FPDFText_GetFillColor"),
            FPDFText_CountRects: load_fn!(lib, "FPDFText_CountRects"),
            FPDFText_GetRect: load_fn!(lib, "FPDFText_GetRect"),
            FPDFText_GetBoundedText: load_fn!(lib, "FPDFText_GetBoundedText"),
            FPDFText_GetText: load_fn!(lib, "FPDFText_GetText"),
            FPDFTextObj_GetTextRenderMode: load_fn!(lib, "FPDFTextObj_GetTextRenderMode"),
            FPDFBitmap_CreateEx: load_fn!(lib, "FPDFBitmap_CreateEx"),
            FPDFBitmap_Destroy: load_fn!(lib, "FPDFBitmap_Destroy"),
            FPDFBitmap_GetWidth: load_fn!(lib, "FPDFBitmap_GetWidth"),
            FPDFBitmap_GetHeight: load_fn!(lib, "FPDFBitmap_GetHeight"),
            FPDFBitmap_GetStride: load_fn!(lib, "FPDFBitmap_GetStride"),
            FPDFBitmap_GetBuffer: load_fn!(lib, "FPDFBitmap_GetBuffer"),
            FPDFBitmap_FillRect: load_fn!(lib, "FPDFBitmap_FillRect"),
            FPDF_RenderPageBitmap: load_fn!(lib, "FPDF_RenderPageBitmap"),
            FPDFTextObj_GetFont: load_fn!(lib, "FPDFTextObj_GetFont"),
            FPDFFont_GetBaseFontName: load_fn!(lib, "FPDFFont_GetBaseFontName"),
            FPDFFont_GetType: load_fn!(lib, "FPDFFont_GetType"),
            FPDFFont_GetIsEmbedded: load_fn!(lib, "FPDFFont_GetIsEmbedded"),
            FPDFFont_GetAscent: load_fn!(lib, "FPDFFont_GetAscent"),
            FPDFFont_GetDescent: load_fn!(lib, "FPDFFont_GetDescent"),
            FPDFFont_GetGlyphWidth: load_fn!(lib, "FPDFFont_GetGlyphWidth"),
            FPDFFont_GetGlyphWidthFromCharCode: load_fn!(lib, "FPDFFont_GetGlyphWidthFromCharCode"),
            FPDFFont_GetGlyphPathFromCharCode: load_fn!(lib, "FPDFFont_GetGlyphPathFromCharCode"),
            FPDFGlyphPath_CountGlyphSegments: load_fn!(lib, "FPDFGlyphPath_CountGlyphSegments"),
            FPDFGlyphPath_GetGlyphPathSegment: load_fn!(lib, "FPDFGlyphPath_GetGlyphPathSegment"),
            FPDFFont_HasToUnicode: load_fn!(lib, "FPDFFont_HasToUnicode"),
            FPDFFont_GetCharGlyphName: load_fn!(lib, "FPDFFont_GetCharGlyphName"),
            FPDFFont_GetEncoding: load_fn!(lib, "FPDFFont_GetEncoding"),
            FPDFFont_GetCharGlyphIndex: load_fn!(lib, "FPDFFont_GetCharGlyphIndex"),
            FPDFFont_GetFontData: load_fn!(lib, "FPDFFont_GetFontData"),

            FPDFBookmark_GetFirstChild: load_fn!(lib, "FPDFBookmark_GetFirstChild"),
            FPDFBookmark_GetNextSibling: load_fn!(lib, "FPDFBookmark_GetNextSibling"),
            FPDFBookmark_GetTitle: load_fn!(lib, "FPDFBookmark_GetTitle"),
            FPDFBookmark_GetDest: load_fn!(lib, "FPDFBookmark_GetDest"),
            FPDFBookmark_GetAction: load_fn!(lib, "FPDFBookmark_GetAction"),
            FPDFAction_GetDest: load_fn!(lib, "FPDFAction_GetDest"),
            FPDFAction_GetURIPath: load_fn!(lib, "FPDFAction_GetURIPath"),
            FPDFLink_Enumerate: load_fn!(lib, "FPDFLink_Enumerate"),
            FPDFLink_GetAction: load_fn!(lib, "FPDFLink_GetAction"),
            FPDFLink_GetAnnotRect: load_fn!(lib, "FPDFLink_GetAnnotRect"),
            FPDFLink_CountQuadPoints: load_fn!(lib, "FPDFLink_CountQuadPoints"),
            FPDFLink_GetQuadPoints: load_fn!(lib, "FPDFLink_GetQuadPoints"),
            FPDFDest_GetDestPageIndex: load_fn!(lib, "FPDFDest_GetDestPageIndex"),
            FPDFDest_GetLocationInPage: load_fn!(lib, "FPDFDest_GetLocationInPage"),

            FPDF_StructTree_GetForPage: load_fn!(lib, "FPDF_StructTree_GetForPage"),
            FPDF_StructTree_Close: load_fn!(lib, "FPDF_StructTree_Close"),
            FPDF_StructTree_CountChildren: load_fn!(lib, "FPDF_StructTree_CountChildren"),
            FPDF_StructTree_GetChildAtIndex: load_fn!(lib, "FPDF_StructTree_GetChildAtIndex"),
            FPDF_StructElement_GetType: load_fn!(lib, "FPDF_StructElement_GetType"),
            FPDF_StructElement_GetAltText: load_fn!(lib, "FPDF_StructElement_GetAltText"),
            FPDF_StructElement_GetTitle: load_fn!(lib, "FPDF_StructElement_GetTitle"),
            FPDF_StructElement_CountChildren: load_fn!(lib, "FPDF_StructElement_CountChildren"),
            FPDF_StructElement_GetChildAtIndex: load_fn!(lib, "FPDF_StructElement_GetChildAtIndex"),
            FPDF_StructElement_GetChildMarkedContentID: load_fn!(
                lib,
                "FPDF_StructElement_GetChildMarkedContentID"
            ),
            FPDF_StructElement_GetMarkedContentID: load_fn!(
                lib,
                "FPDF_StructElement_GetMarkedContentID"
            ),
            FPDF_StructElement_GetMarkedContentIdCount: load_fn!(
                lib,
                "FPDF_StructElement_GetMarkedContentIdCount"
            ),
            FPDF_StructElement_GetMarkedContentIdAtIndex: load_fn!(
                lib,
                "FPDF_StructElement_GetMarkedContentIdAtIndex"
            ),

            _lib: lib,
        })
    }
}

/// Shared library file name for the current platform.
fn dylib_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libpdfium.dylib"
    }
    #[cfg(target_os = "windows")]
    {
        "pdfium.dll"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "libpdfium.so"
    }
}

/// Get the directory containing the current shared library (the .pyd/.so/.node/.dll
/// that this code is compiled into). This lets us find sibling files like pdfium.dll
/// that are bundled next to the native extension in Python wheels, Node packages, etc.
fn self_dir() -> Option<PathBuf> {
    // Use a static function in this module as the probe address.
    let addr = self_dir as *const ();

    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleExW(
                dwFlags: u32,
                lpModuleName: *const u8,
                phModule: *mut *mut std::ffi::c_void,
            ) -> i32;
            fn GetModuleFileNameW(
                hModule: *mut std::ffi::c_void,
                lpFilename: *mut u16,
                nSize: u32,
            ) -> u32;
        }

        const FROM_ADDRESS: u32 = 0x00000004;
        const UNCHANGED_REFCOUNT: u32 = 0x00000002;

        unsafe {
            let mut module = std::ptr::null_mut();
            if GetModuleHandleExW(
                FROM_ADDRESS | UNCHANGED_REFCOUNT,
                addr as *const u8,
                &mut module,
            ) == 0
            {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let len = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32);
            if len == 0 || len >= buf.len() as u32 {
                return None;
            }
            let path = PathBuf::from(OsString::from_wide(&buf[..len as usize]));
            path.parent().map(|p| p.to_path_buf())
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        #[repr(C)]
        struct DlInfo {
            dli_fname: *const std::os::raw::c_char,
            dli_fbase: *mut std::ffi::c_void,
            dli_sname: *const std::os::raw::c_char,
            dli_saddr: *mut std::ffi::c_void,
        }

        unsafe extern "C" {
            fn dladdr(addr: *const std::ffi::c_void, info: *mut DlInfo) -> i32;
        }

        unsafe {
            let mut info: DlInfo = std::mem::zeroed();
            if dladdr(addr as *const std::ffi::c_void, &mut info) != 0 && !info.dli_fname.is_null()
            {
                let cstr = std::ffi::CStr::from_ptr(info.dli_fname);
                let path = PathBuf::from(cstr.to_string_lossy().as_ref());
                return path.parent().map(|p| p.to_path_buf());
            }
            None
        }
    }
}

/// Search paths for the pdfium shared library, in priority order.
fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let name = dylib_name();

    // 1. Runtime env var override (directory containing the shared library)
    if let Ok(dir) = std::env::var("PDFIUM_LIB_PATH") {
        paths.push(PathBuf::from(&dir).join(name));
    }

    // 2. Compile-time baked path from build.rs
    if !PDFIUM_LIB_DIR.is_empty() {
        let lib_dir = PathBuf::from(PDFIUM_LIB_DIR);
        paths.push(lib_dir.join(name));

        // On Windows, pdfium-binaries puts the DLL in bin/, not lib/
        #[cfg(target_os = "windows")]
        if let Some(parent) = lib_dir.parent() {
            paths.push(parent.join("bin").join(name));
        }
    }

    // 3. Next to the native extension (Python .pyd/.so, Node .node, etc.)
    //    Uses dladdr (Unix) / GetModuleHandleExW (Windows) to find our own module path.
    if let Some(dir) = self_dir() {
        paths.push(dir.join(name));
    }

    // 4. Next to the current executable
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        paths.push(exe_dir.join(name));
    }

    // 5. Bare library name (system search paths / LD_LIBRARY_PATH / DYLD_LIBRARY_PATH / PATH)
    paths.push(PathBuf::from(name));

    paths
}

/// Load the pdfium shared library from a specific path.
pub fn load(lib_path: &Path) -> Result<(), String> {
    if BINDINGS.get().is_some() {
        return Ok(());
    }
    let lib = unsafe { Library::new(lib_path) }
        .map_err(|e| format!("failed to load pdfium from {}: {e}", lib_path.display()))?;
    let bindings =
        PdfiumBindings::load(lib).map_err(|e| format!("failed to resolve pdfium symbols: {e}"))?;
    let _ = BINDINGS.set(bindings);
    Ok(())
}

/// Load the pdfium shared library from default search paths.
///
/// Search order:
/// 1. `PDFIUM_LIB_PATH` env var (directory containing the shared library)
/// 2. Compile-time cached download path
/// 3. System library search paths
pub fn load_default() -> Result<(), String> {
    if BINDINGS.get().is_some() {
        return Ok(());
    }

    let paths = search_paths();
    let mut last_err = String::from("no search paths configured");

    for path in &paths {
        match unsafe { Library::new(path) } {
            Ok(lib) => match PdfiumBindings::load(lib) {
                Ok(bindings) => {
                    let _ = BINDINGS.set(bindings);
                    return Ok(());
                }
                Err(e) => {
                    last_err = format!(
                        "failed to resolve pdfium symbols from {}: {e}",
                        path.display()
                    );
                }
            },
            Err(e) => {
                last_err = format!("{}: {e}", path.display());
            }
        }
    }

    Err(format!(
        "could not find pdfium shared library. Last error: {last_err}. \
         Set PDFIUM_LIB_PATH to the directory containing {}",
        dylib_name()
    ))
}

/// Get a reference to the loaded pdfium bindings.
///
/// # Panics
/// Panics if `load()` or `load_default()` has not been called successfully.
pub fn pdfium() -> &'static PdfiumBindings {
    BINDINGS
        .get()
        .expect("pdfium not loaded — call pdfium_sys::dynamic::load_default() first")
}
