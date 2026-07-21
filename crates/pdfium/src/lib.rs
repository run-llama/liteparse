mod bitmap;
mod document;
mod error;
mod font;
mod library;
mod page;
mod struct_tree;
mod text_page;
mod types;

pub use bitmap::Bitmap;
pub use document::{Document, OutlineEntry};
pub use error::PdfiumError;
pub use font::{Font, FontType};
pub use library::Library;
pub use page::{
    ImageBounds, ImageObjectInfo, ImageObjects, Page, PathObject, PathSegment, PdfLink,
    SegmentKind, ViewportTransform,
};
pub use struct_tree::StructNode;
pub use text_page::{TextChar, TextCharIter, TextPage};
pub use types::*;

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
