mod bitmap;
mod document;
mod error;
mod font;
mod library;
mod page;
mod page_object;
mod struct_tree;
mod text_page;
mod types;
mod user_unit;

pub use bitmap::{Bitmap, BitmapFormat};
pub use document::{Document, FormEnvironment, OutlineEntry, SignatureSummary, XfaPacket};
pub use error::PdfiumError;
pub use font::{Font, FontType};
pub use library::Library;
pub use page::{
    FormPageGuard, FormWidgetsGuard, HiddenAnnotations, ImageBounds, ImageObjectInfo, ImageObjects,
    Page, PathObject, PathSegment, PdfAnnotation, PdfFormField, PdfLink, SegmentKind,
    ViewportTransform,
};
pub use page_object::{ImageMetadata, PageObject, PageObjectKind, PathDrawMode, RawPathSegment};
pub use struct_tree::{StructNode, StructureAttributeValue, StructureElement, StructureTree};
pub use text_page::{TextChar, TextCharIter, TextPage};
pub use types::*;

/// Raw FFI layer, re-exported for callers that need to hold raw handles
/// (e.g. `FPDF_PAGEOBJECT`) returned by the safe wrappers.
pub use pdfium_sys;

/// Unified FFI call macro. On wasm, calls pdfium_sys extern functions directly.
/// On non-wasm, calls through the runtime-loaded function pointers.
#[cfg(not(target_arch = "wasm32"))]
macro_rules! ffi {
    ($fn_name:ident($($args:expr),* $(,)?)) => {
        (pdfium_sys::dynamic::pdfium().$fn_name)($($args),*)
    }
}

#[cfg(target_arch = "wasm32")]
macro_rules! ffi {
    ($fn_name:ident($($args:expr),* $(,)?)) => {
        pdfium_sys::$fn_name($($args),*)
    }
}

pub(crate) use ffi;
