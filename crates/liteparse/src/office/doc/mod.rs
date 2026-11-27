//! Binary Word inspection. Raw stories retain source control characters and field codes.
//! Formatting, revision visibility and page layout are not interpreted yet.
pub mod autonum;
pub mod bookmarks;
pub mod capitalization;
pub mod comments;
/// Root-level streams of an OLE compound file; the PowerPoint reader (`office::ppt`) shares it.
pub mod compound;
mod document;
pub mod docx_export;
pub mod equation_fields;
pub mod extraction_summary;
mod field_display_visibility;
pub mod field_inspection;
pub mod fields;
pub mod form_fields;
pub mod formatting;
pub mod formatting_inspection;
pub mod geometry_text;
pub mod headers;
pub mod hidden_visibility;
pub mod linked_headers;
pub mod linked_notes;
pub mod list_projection;
pub mod lists;
pub mod notes;
pub mod numbering_revision;
pub mod paragraph_end_formatting;
pub mod paragraph_formatting;
pub mod paragraph_references;
pub mod pictures;
pub mod property_revision;
pub mod render_model;
pub mod revision_projection;
pub mod ruby_fields;
mod source_literal_projection;
pub mod structure;
pub mod styles;
pub mod symbol_fields;
pub mod symbols;
pub mod table_structure;
pub mod textbox_anchors;
pub mod textbox_flow;
mod transcription;
pub mod visibility;

pub use document::{Inspection, Story, inspect};
pub use transcription::{Diagnostic, SourceSegment, TranscribedStory, Transcription, transcribe};
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(clippy::error_impl_error, reason = "the crate's public error type")]
pub enum Error {
    Invalid(&'static str),
    Unsupported(&'static str),
    Limit(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => write!(f, "invalid: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Limit(s) => write!(f, "limit: {s}"),
        }
    }
}
impl std::error::Error for Error {}

fn bytes(b: &[u8], off: usize, len: usize) -> Result<&[u8]> {
    b.get(
        off..off
            .checked_add(len)
            .ok_or(Error::Invalid("offset overflow"))?,
    )
    .ok_or(Error::Invalid("truncated structure"))
}
fn u16_at(b: &[u8], off: usize) -> Result<u16> {
    let s = bytes(b, off, 2)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], off: usize) -> Result<u32> {
    let s = bytes(b, off, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[cfg(any(test, test))]
#[doc(hidden)]
pub mod fixtures;
#[cfg(test)]
mod tests;

pub mod ordinary_symbols;
