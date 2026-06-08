use serde::{Deserialize, Serialize};

/// Configuration for LiteParse document parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteParseConfig {
    /// OCR language code (Tesseract format: "eng", "fra", "deu", etc.).
    pub ocr_language: String,
    /// Whether OCR is enabled. When true, runs on text-sparse pages and embedded images.
    pub ocr_enabled: bool,
    /// HTTP OCR server URL (uses Tesseract if not provided)
    pub ocr_server_url: Option<String>,
    /// Path to tessdata directory. Falls back to TESSDATA_PREFIX env var if not set.
    pub tessdata_path: Option<String>,
    /// Maximum number of pages to parse.
    pub max_pages: usize,
    /// Specific pages to parse (e.g., "1-5,10,15-20"). None means all pages.
    pub target_pages: Option<String>,
    /// DPI for rendering pages (used for OCR and screenshots).
    pub dpi: f32,
    /// Output format.
    pub output_format: OutputFormat,
    /// Keep very small text that would normally be filtered out.
    pub preserve_very_small_text: bool,
    /// Password for encrypted/protected documents.
    pub password: Option<String>,
    /// Suppress progress output.
    pub quiet: bool,
    /// Number of concurrent OCR workers. Defaults to (number of CPU cores - 1), minimum 1.
    pub num_workers: usize,
    /// Inline embedded PDF images into page text as markdown data URI images.
    /// Disabled by default because it can make text/JSON output very large.
    pub inline_images: bool,
}

/// Supported output formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Text,
}

impl Default for LiteParseConfig {
    fn default() -> Self {
        Self {
            ocr_language: "eng".to_string(),
            ocr_enabled: true,
            ocr_server_url: None,
            tessdata_path: None,
            max_pages: 1000,
            target_pages: None,
            dpi: 150.0,
            output_format: OutputFormat::Json,
            preserve_very_small_text: false,
            password: None,
            quiet: false,
            num_workers: default_num_workers(),
            inline_images: false,
        }
    }
}

/// Returns the default number of OCR workers: CPU cores - 1, minimum 1.
fn default_num_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).max(1))
        .unwrap_or(1)
}

#[doc(hidden)]
pub fn parse_target_pages(s: &str) -> Result<Vec<u32>, String> {
    let mut pages = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.contains('-') {
            let bounds: Vec<&str> = part.splitn(2, '-').collect();
            let start: u32 = bounds[0]
                .trim()
                .parse()
                .map_err(|_| format!("invalid page number: {}", bounds[0]))?;
            let end: u32 = bounds[1]
                .trim()
                .parse()
                .map_err(|_| format!("invalid page number: {}", bounds[1]))?;
            if start > end {
                return Err(format!("invalid range: {}-{}", start, end));
            }
            for p in start..=end {
                pages.push(p);
            }
        } else {
            let p: u32 = part
                .parse()
                .map_err(|_| format!("invalid page number: {}", part))?;
            pages.push(p);
        }
    }
    pages.sort();
    pages.dedup();
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_target_pages() {
        assert_eq!(
            parse_target_pages("1-5,10,15-20").unwrap(),
            vec![1, 2, 3, 4, 5, 10, 15, 16, 17, 18, 19, 20]
        );
        assert_eq!(parse_target_pages("3").unwrap(), vec![3]);
        assert_eq!(parse_target_pages("1,1,2").unwrap(), vec![1, 2]);
        assert!(parse_target_pages("5-3").is_err());
        assert!(parse_target_pages("abc").is_err());
    }

    #[test]
    fn test_parse_target_pages_with_whitespace() {
        assert_eq!(parse_target_pages(" 1 , 2 - 4 ").unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_parse_target_pages_single_range() {
        assert_eq!(parse_target_pages("2-2").unwrap(), vec![2]);
    }

    #[test]
    fn test_default_config() {
        let c = LiteParseConfig::default();
        assert_eq!(c.ocr_language, "eng");
        assert!(c.ocr_enabled);
        assert_eq!(c.max_pages, 1000);
        assert_eq!(c.dpi, 150.0);
        assert_eq!(c.output_format, OutputFormat::Json);
        assert!(!c.preserve_very_small_text);
        assert!(!c.quiet);
        assert!(!c.inline_images);
        assert!(c.password.is_none());
    }

    #[test]
    fn test_output_format_lowercase_serde() {
        let s = serde_json::to_string(&OutputFormat::Json).unwrap();
        assert_eq!(s, "\"json\"");
        let back: OutputFormat = serde_json::from_str("\"text\"").unwrap();
        assert_eq!(back, OutputFormat::Text);
    }

    #[test]
    fn test_config_roundtrip() {
        let c = LiteParseConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: LiteParseConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.ocr_language, c.ocr_language);
        assert_eq!(back.output_format, c.output_format);
    }
}
