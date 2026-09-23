//! LiteParse — open-source PDF parsing with spatial text extraction, OCR, and bounding boxes.
//!
//! This crate is the core Rust library. Language bindings for Node.js, Python,
//! and WebAssembly re-export the same types with language-idiomatic wrappers.
//!
//! [`LiteParse::parse`] runs a fixed pipeline; the [`stages`] module exposes
//! each step of it as a public function for callers that want to sequence
//! the pipeline themselves.
//!

// ── Public API re-exports ──────────────────────────────────────────────
pub use config::{DEFAULT_PAGE_BATCH_SIZE, LiteParseConfig, OutputFormat};
pub use error::LiteParseError;
#[cfg(not(target_arch = "wasm32"))]
pub use font_db_resolver::FontDbResolver;
pub use glyph_resolver::{GLYPH_RESOLVER_FONT_SIZE, GlyphResolver};
pub use parser::{LiteParse, ParseBatch, ParseResult, ParseSession, ScreenshotResult};
pub use raw_text::{RawTextItem, append_raw_widget_text_items, extract_raw_text_items};
pub use search::{SearchOptions, search_items};
pub use types::{DocumentMetadata, ParsedPage, TextItem, WordBox};

// ── Modules with user-facing types (visible in docs) ───────────────────
pub mod config;
pub mod error;
pub mod glyph_resolver;
pub mod layout;
pub mod parser;
pub mod raw_text;
pub mod search;
pub mod stages;
pub mod types;

// ── Internal modules (available for binding crates, hidden from docs) ──
#[cfg(not(target_arch = "wasm32"))]
mod acroform_repair;
mod bidi;
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub mod conversion;
mod document_metadata;
#[doc(hidden)]
pub mod extract;
#[doc(hidden)]
pub mod figure_cluster;
#[doc(hidden)]
pub mod font_cmap;
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub mod font_db_resolver;
#[doc(hidden)]
pub mod glyph_names;
#[doc(hidden)]
pub mod markdown_layout;
#[doc(hidden)]
pub mod ocr;

#[doc(hidden)]
pub mod ocr_merge;
#[doc(hidden)]
pub mod output;
#[doc(hidden)]
pub mod projection;
#[doc(hidden)]
pub mod render;
