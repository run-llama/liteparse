//! Legacy binary PowerPoint (.ppt, [MS-PPT]) to PPTX conversion.
//!
//! [`convert`] reads the `PowerPoint Document` stream through its persist directory and
//! writes an in-memory .pptx that `office::pptx` renders unchanged, the way `office::doc`'s
//! `docx_export` feeds `office::docx`. This milestone carries slide size, slide order and
//! text at its shape anchors. Unsupported structures become diagnostics; only an
//! unreadable container, a missing document, or encryption is an error.

mod dates;
mod metro;
mod native;
mod persist;
mod pptx;
mod props;
mod records;
mod slides;
mod text;

use crate::office::doc::compound::Compound;

pub struct Converted {
    pub pptx: Vec<u8>,
    /// One line per skipped or repaired structure, with a count.
    pub diagnostics: Vec<String>,
}

#[derive(Debug)]
#[expect(clippy::error_impl_error, reason = "the crate's public error type")]
pub enum Error {
    /// The OLE compound container is unreadable.
    Container(crate::office::doc::Error),
    Encrypted,
    Invalid(&'static str),
    Package(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Container(e) => write!(f, "compound file: {e}"),
            Self::Encrypted => write!(f, "encrypted presentation"),
            Self::Invalid(s) => write!(f, "invalid: {s}"),
            Self::Package(s) => write!(f, "writing pptx: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub use slides::Options;

impl Default for Options {
    fn default() -> Self {
        Options {
            metro: true,
            metro_shapes: true,
            now: None,
        }
    }
}

pub fn convert(data: &[u8]) -> Result<Converted, Error> {
    convert_with(data, Options::default())
}

pub fn convert_with(data: &[u8], opts: Options) -> Result<Converted, Error> {
    let cfb = Compound::open(data).map_err(Error::Container)?;
    let doc = cfb
        .stream("PowerPoint Document")
        .map_err(Error::Container)?;
    let current_user = cfb.stream("Current User").ok();
    let pictures = cfb.stream("Pictures").unwrap_or_default();
    let mut diagnostics = Vec::new();
    let persist = persist::read(current_user.as_deref(), &doc, &mut diagnostics)?;
    let mut skipped = slides::Skipped::default();
    let deck = slides::read(&doc, &pictures, &persist, opts, &mut skipped)?;
    skipped.report(&mut diagnostics);
    let pptx = pptx::write(&deck).map_err(|e| Error::Package(e.to_string()))?;
    Ok(Converted { pptx, diagnostics })
}

#[cfg(any(test, test))]
#[doc(hidden)]
pub mod fixtures;
#[cfg(test)]
mod tests;
