//! Document-level failures, mapped onto the worker's existing error codes so the
//! index file reads the same as a `liteparse-read` failure.

use std::fmt;

#[derive(Debug)]
pub enum DocError {
    /// Password-encrypted or IRM/DRM-protected package.
    Protected,
    /// Not a readable OOXML container, or no presentation part.
    Broken(String),
    /// Text was transferred lossily (high U+FFFD density in the XML).
    Corrupt(String),
    MissingPart(String),
    LimitExceeded(String),
    Timeout,
    Io(String),
}

pub type DocResult<T> = Result<T, DocError>;

impl DocError {
    /// The code written to `index.json` `error`. These strings are the ones the
    /// worker already maps to user-facing failures.
    pub fn code(&self) -> &'static str {
        match self {
            DocError::Protected => "PDF_IS_PROTECTED",
            DocError::Broken(_) | DocError::MissingPart(_) => "OFFICE_FILE_IS_BROKEN",
            DocError::Corrupt(_) => "FILE_IS_CORRUPT",
            DocError::LimitExceeded(_) => "OFFICE_FILE_IS_BROKEN",
            DocError::Timeout => "DOCUMENT_CONVERSION_TIMEOUT",
            DocError::Io(_) => "DOCUMENT_CONVERSION_ERROR",
        }
    }
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocError::Protected => write!(f, "package is encrypted or protected"),
            DocError::Broken(m) => write!(f, "broken package: {m}"),
            DocError::Corrupt(m) => write!(f, "corrupt package: {m}"),
            DocError::MissingPart(p) => write!(f, "missing part {p}"),
            DocError::LimitExceeded(m) => write!(f, "limit exceeded: {m}"),
            DocError::Timeout => write!(f, "document budget exceeded"),
            DocError::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for DocError {}

impl From<std::io::Error> for DocError {
    fn from(e: std::io::Error) -> Self {
        DocError::Io(e.to_string())
    }
}
