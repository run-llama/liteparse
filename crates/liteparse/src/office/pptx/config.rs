//! The request contract: a superset of the worker's `PdfiumConfig`
//! (`llamaparse/worker/src/utils/pdfiumConfig.ts`). Inherited fields keep their
//! names and meaning; `presentation`, `render`, `pdf` and `limits` are pptx-only.
//! See `llamaparse/pptx-native/SPEC.md` §2.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub verbose: bool,
    /// 0-based page ranges over emitted pages (`"0-2,5"`), same grammar as the worker.
    pub target_pages: Option<String>,
    /// 0 means no limit.
    pub max_pages: u32,
    pub jpeg_quality: Option<u8>,
    pub png_compress_level: Option<u8>,
    pub workers: Option<u32>,
    pub annotation: Toggle,
    pub image: ImageConfig,
    pub path: PathConfig,
    pub screenshot: ScreenshotConfig,
    pub text: Toggle,
    // Accepted for contract compatibility; a pptx has no tagged structure or forms.
    pub structtree: Toggle,
    pub acroform: Toggle,
    pub xfa: Toggle,
    pub presentation: PresentationConfig,
    pub render: RenderConfig,
    pub pdf: PdfConfig,
    pub limits: Limits,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Toggle {
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageConfig {
    pub enabled: bool,
    pub max_per_page: u32,
    pub output_dir: Option<String>,
    pub one_index_pages: bool,
    pub metadata_only: bool,
}

impl Default for ImageConfig {
    fn default() -> Self {
        ImageConfig {
            enabled: false,
            max_per_page: 30,
            output_dir: None,
            one_index_pages: false,
            metadata_only: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PathConfig {
    pub enabled: bool,
    pub max_per_page: u32,
}

impl Default for PathConfig {
    fn default() -> Self {
        PathConfig {
            enabled: false,
            max_per_page: 100_000,
        }
    }
}

/// Crop box as fractions of the page, as the worker sends it.
#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(default)]
pub struct CropBox {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenshotConfig {
    pub enabled: bool,
    pub format: String,
    pub dpi: f32,
    /// 0 means "pick the default for the dpi" (1600, or 3200 above 200 dpi).
    pub max_edge: u32,
    pub output_dir: Option<String>,
    pub bbox: Option<CropBox>,
    pub detect_rectangles: bool,
    pub detect_lines: bool,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        ScreenshotConfig {
            enabled: false,
            format: "jpg".into(),
            dpi: 200.0,
            max_edge: 0,
            output_dir: None,
            bbox: None,
            detect_rectangles: false,
            detect_lines: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum OutOfBounds {
    #[default]
    Clip,
    Expand,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresentationConfig {
    pub include_hidden_slides: bool,
    pub out_of_bounds_content: OutOfBounds,
    pub skip_embedded_data: bool,
    pub speaker_notes: bool,
    pub charts: bool,
    pub tables: bool,
    pub sections: bool,
    pub comments: bool,
    pub shapes: bool,
}

impl Default for PresentationConfig {
    fn default() -> Self {
        PresentationConfig {
            include_hidden_slides: false,
            out_of_bounds_content: OutOfBounds::Clip,
            skip_embedded_data: false,
            speaker_notes: true,
            charts: true,
            tables: true,
            sections: true,
            comments: true,
            shapes: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RenderConfig {
    /// Extra font directories, searched before the system ones.
    pub font_dirs: Vec<String>,
    /// Skip system font directories (production images ship a fixed font set).
    pub no_system_fonts: bool,
    pub font_substitutions: std::collections::HashMap<String, String>,
    pub use_embedded_fonts: bool,
    pub metafiles: MetafileMode,
    pub slide_timeout_ms: u64,
    pub document_budget_ms: u64,
    /// docx-read only: lay out text in Microsoft fonts that are not installed
    /// with their tabled metrics (line height, advances, kerning).
    pub word_metrics: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        RenderConfig {
            font_dirs: Vec::new(),
            no_system_fonts: false,
            font_substitutions: Default::default(),
            use_embedded_fonts: true,
            metafiles: MetafileMode::Render,
            slide_timeout_ms: 30_000,
            document_budget_ms: 150_000,
            word_metrics: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum MetafileMode {
    #[default]
    Render,
    Skip,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum PdfSource {
    #[default]
    Screenshot,
    Render,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PdfConfig {
    pub enabled: bool,
    pub output_path: Option<String>,
    pub source: PdfSource,
    pub dpi: f32,
    pub format: String,
}

impl Default for PdfConfig {
    fn default() -> Self {
        PdfConfig {
            enabled: false,
            output_path: None,
            source: PdfSource::Screenshot,
            dpi: 150.0,
            format: "jpeg".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    pub max_zip_entries: u32,
    /// Largest inflated size of any single part.
    pub max_part_bytes: u64,
    pub max_image_pixels: u64,
    pub max_shapes_per_slide: u32,
    pub max_slides: u32,
    /// Peak resident memory (bytes) the decoder process may reach before it exits
    /// with a memory-limit code; 0 leaves it uncapped. Enforced by docx-read.
    pub max_rss_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_zip_entries: 50_000,
            max_part_bytes: 512 * 1024 * 1024,
            max_image_pixels: 128 * 1024 * 1024,
            max_shapes_per_slide: 20_000,
            max_slides: 10_000,
            max_rss_bytes: 0,
        }
    }
}

/// Parses the worker's 0-based page range grammar: `"0,2-4"`. Unparseable pieces
/// are ignored, as in `liteparse-read`.
pub fn parse_target_pages(spec: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for piece in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((a, b)) = piece.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                out.extend(a.min(b)..=a.max(b));
            }
        } else if let Ok(v) = piece.parse::<u32>() {
            out.push(v);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

impl Config {
    pub fn jpeg_quality(&self) -> u8 {
        self.jpeg_quality.unwrap_or(90).clamp(1, 100)
    }

    pub fn png_level(&self) -> u8 {
        self.png_compress_level.unwrap_or(6).min(9)
    }
}

#[cfg(test)]
mod tests;
