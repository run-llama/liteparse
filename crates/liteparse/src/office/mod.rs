//! Native Word and PowerPoint support (`.docx`, `.doc`, `.pptx`, `.ppt`).
//!
//! These formats never go through LibreOffice. The decoders below read the
//! file, lay it out the way Word or PowerPoint would, and report every page's
//! text runs, vector graphics, pictures and (on request) a rendered
//! screenshot. [`adapter`] turns that into liteparse [`crate::types::Page`]s,
//! which then go through the same projection and markdown passes as PDF text.
//!
//! - [`docx`]: WordprocessingML layout and pagination. Binary `.doc` input is
//!   first converted to `.docx` in memory by [`doc`].
//! - [`pptx`]: PresentationML slides, charts, SmartArt and tables. Binary
//!   `.ppt` input is first converted to `.pptx` in memory by [`ppt`]. Its
//!   scene, renderer and font system are shared with [`docx`].
//!
//! The decoders come from the LlamaParse platform repository. Their module
//! READMEs and notes are kept beside the code.

// The decoders were written against a stricter lint set; their `#[expect]`s
// name lints this crate does not enable.
#![allow(unfulfilled_lint_expectations)]

pub mod adapter;
pub mod doc;
pub mod docx;
pub mod ppt;
pub mod pptx;

pub use adapter::{Decoded, Kind, Request, ScreenshotRequest, decode};
