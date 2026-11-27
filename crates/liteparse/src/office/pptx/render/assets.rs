//! Immutable image bytes supplied by the source document decoder.
use crate::office::pptx::{document::Document, error::DocResult};

/// Resolves scene image keys within one document.
///
/// Implementations enforce their source's allocation and input limits. Reads return
/// an error for missing or inaccessible assets; `read_head` returns at most `max`
/// bytes. Keys and bytes must remain stable for the lifetime of the image and layer
/// caches. Caches belong to one document and must not be reused with another source.
pub trait RenderAssets {
    fn read(&self, part: &str) -> DocResult<Vec<u8>>;
    fn read_head(&self, part: &str, max: u64) -> DocResult<Vec<u8>>;
    fn part_size(&self, part: &str) -> Option<u64>;
}

impl RenderAssets for Document {
    fn read(&self, part: &str) -> DocResult<Vec<u8>> {
        self.pkg.read(part)
    }

    fn read_head(&self, part: &str, max: u64) -> DocResult<Vec<u8>> {
        self.pkg.read_head(part, max)
    }

    fn part_size(&self, part: &str) -> Option<u64> {
        self.pkg.part_size(part)
    }
}

#[cfg(test)]
mod tests;
