use std::path::{Path, PathBuf};
use std::pin::Pin;

use super::{OcrEngine, OcrOptions, OcrResult};
use tesseract_rs::{TessPageIteratorLevel, TessPageSegMode, TesseractAPI};

const TESSDATA_BASE_URL: &str = "https://github.com/tesseract-ocr/tessdata_best/raw/main";

pub struct TesseractOcrEngine {
    tessdata_path: Option<String>,
}

impl TesseractOcrEngine {
    pub fn new(tessdata_path: Option<String>) -> Self {
        Self { tessdata_path }
    }

    fn normalize_language_code(lang: &str) -> &str {
        match lang.to_lowercase().trim() {
            "en" => "eng",
            "fr" => "fra",
            "de" => "deu",
            "es" => "spa",
            "it" => "ita",
            "pt" => "por",
            "ru" => "rus",
            "zh" | "zh-cn" => "chi_sim",
            "zh-tw" => "chi_tra",
            "ja" => "jpn",
            "ko" => "kor",
            "ar" => "ara",
            "hi" => "hin",
            "th" => "tha",
            "vi" => "vie",
            _ => lang.trim(),
        }
    }

    /// Normalize a tesseract language spec, which may be a single code or a
    /// `+`-separated list (e.g. `ita+eng`). Each component is normalized
    /// independently and rejoined with `+`.
    fn normalize_language(lang: &str) -> String {
        lang.split('+')
            .filter(|part| !part.trim().is_empty())
            .map(Self::normalize_language_code)
            .collect::<Vec<_>>()
            .join("+")
    }
}

impl OcrEngine for TesseractOcrEngine {
    fn name(&self) -> &str {
        "tesseract"
    }

    // Tesseract binarizes internally (Leptonica/Otsu on luma), so a grayscale
    // buffer is equivalent input at a third of the memory.
    fn prefers_grayscale(&self) -> bool {
        true
    }

    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image_data: &'c [u8],
        width: u32,
        height: u32,
        options: &'b OcrOptions,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<OcrResult>, Box<dyn std::error::Error + Send + Sync>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            let language = Self::normalize_language(&options.language);

            let api = TesseractAPI::new();

            // Determine tessdata path: explicit config > TESSDATA_PREFIX env > tesseract-rs default
            let tessdata_path = self
                .tessdata_path
                .clone()
                .or_else(|| std::env::var("TESSDATA_PREFIX").ok());

            let resolved_path = tessdata_path.unwrap_or_else(default_tessdata_dir);
            for code in language.split('+') {
                ensure_traineddata(Path::new(&resolved_path), code).await?;
            }
            api.init(&resolved_path, &language)?;

            // Match the tesseract CLI's default page segmentation mode (PSM_AUTO,
            // i.e. 3). The C++ library's own default when SetPageSegMode is never
            // called is PSM_SINGLE_BLOCK (6), which assumes the image is a single
            // uniform block of text and performs poorly on full-page layouts.
            api.set_page_seg_mode(TessPageSegMode::PSM_AUTO)?;

            // Channels inferred from the buffer: 1 = grayscale, 3 = RGB.
            let bytes_per_pixel = if width > 0 && height > 0 {
                (image_data.len() / (width as usize * height as usize)).clamp(1, 4) as i32
            } else {
                3
            };
            let bytes_per_line = width as i32 * bytes_per_pixel;
            api.set_image(
                image_data,
                width as i32,
                height as i32,
                bytes_per_pixel,
                bytes_per_line,
            )?;

            // Tesseract can't infer DPI from a raw RGB buffer (there's no image
            // header), so it falls back to a guess and warns. Tell it the actual
            // render resolution so its internal point-size/threshold heuristics are
            // correct. Must come after set_image, which resets the resolution.
            api.set_source_resolution(options.dpi.round() as i32)?;

            api.recognize()?;

            let iter = api.get_iterator()?;

            let mut results = Vec::new();
            loop {
                if let Ok((text, left, top, right, bottom, confidence)) =
                    iter.get_word_with_bounds()
                {
                    // tesseract-rs returns confidence 0-100, normalize to 0-1
                    let conf = confidence / 100.0;

                    // Filter low confidence (below 30%, matching TS behavior)
                    if conf > 0.3 && !text.trim().is_empty() {
                        results.push(OcrResult {
                            text,
                            bbox: [left as f32, top as f32, right as f32, bottom as f32],
                            confidence: conf,
                            polygon: None,
                            words: Vec::new(),
                        });
                    }
                }

                match iter.next(TessPageIteratorLevel::RIL_WORD) {
                    Ok(true) => continue,
                    _ => break,
                }
            }

            Ok(results)
        })
    }
}

/// Default tessdata directory. Matches the locations used by tesseract-rs's
/// build-tesseract feature so any traineddata it downloaded at build time is
/// also picked up here.
fn default_tessdata_dir() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/Library/Application Support/tesseract-rs/tessdata", home);
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/.tesseract-rs/tessdata", home);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var("APPDATA").ok().or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .map(|p| format!("{}\\AppData\\Roaming", p))
        }) {
            return format!("{}\\tesseract-rs\\tessdata", base);
        }
    }
    "tessdata".to_string()
}

/// Ensure `<lang>.traineddata` exists in `dir`. If missing, downloads it from
/// the upstream `tessdata_best` repo — mirroring tesseract.js's first-use
/// download behavior. Concurrent calls are safe via an atomic rename.
async fn ensure_traineddata(
    dir: &Path,
    language: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filename = format!("{}.traineddata", language);
    let final_path: PathBuf = dir.join(&filename);
    if final_path.exists() {
        return Ok(());
    }

    tokio::fs::create_dir_all(dir).await?;

    let url = format!("{}/{}", TESSDATA_BASE_URL, filename);
    let response = reqwest::get(&url).await?;
    if !response.status().is_success() {
        return Err(format!(
            "failed to download tessdata for language \"{}\" from {}: HTTP {}",
            language,
            url,
            response.status()
        )
        .into());
    }
    let bytes = response.bytes().await?;

    // Write to a temp file in the same directory, then atomically rename.
    // This makes concurrent first-use safe: whichever rename lands last wins,
    // and partial files never appear at the final path.
    let tmp_path = dir.join(format!(
        "{}.traineddata.tmp.{}",
        language,
        std::process::id()
    ));
    tokio::fs::write(&tmp_path, &bytes).await?;
    if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
        // If another task beat us to it, the file exists — that's fine.
        let _ = tokio::fs::remove_file(&tmp_path).await;
        if !final_path.exists() {
            return Err(e.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_language_known_codes() {
        assert_eq!(TesseractOcrEngine::normalize_language("en"), "eng");
        assert_eq!(TesseractOcrEngine::normalize_language("EN"), "eng");
        assert_eq!(TesseractOcrEngine::normalize_language(" fr "), "fra");
        assert_eq!(TesseractOcrEngine::normalize_language("zh"), "chi_sim");
        assert_eq!(TesseractOcrEngine::normalize_language("zh-tw"), "chi_tra");
        assert_eq!(TesseractOcrEngine::normalize_language("ja"), "jpn");
    }

    #[test]
    fn test_normalize_language_passthrough_for_unknown() {
        assert_eq!(TesseractOcrEngine::normalize_language("eng"), "eng");
        assert_eq!(TesseractOcrEngine::normalize_language("xyz"), "xyz");
    }

    #[test]
    fn test_normalize_language_multi() {
        // `+`-separated specs normalize each component independently.
        assert_eq!(TesseractOcrEngine::normalize_language("ita+eng"), "ita+eng");
        assert_eq!(TesseractOcrEngine::normalize_language("it+en"), "ita+eng");
        assert_eq!(
            TesseractOcrEngine::normalize_language(" it + en "),
            "ita+eng"
        );
        assert_eq!(TesseractOcrEngine::normalize_language("eng+xyz"), "eng+xyz");
        // Empty components (leading/trailing/double `+`) are dropped.
        assert_eq!(TesseractOcrEngine::normalize_language("eng+"), "eng");
        assert_eq!(TesseractOcrEngine::normalize_language("+eng"), "eng");
        assert_eq!(
            TesseractOcrEngine::normalize_language("ita++eng"),
            "ita+eng"
        );
    }

    #[test]
    fn test_engine_name() {
        let e = TesseractOcrEngine::new(None);
        assert_eq!(e.name(), "tesseract");
    }

    #[test]
    fn test_new_stores_tessdata_path() {
        let e = TesseractOcrEngine::new(Some("/custom/tessdata".to_string()));
        assert_eq!(e.tessdata_path.as_deref(), Some("/custom/tessdata"));
    }

    #[test]
    fn test_default_tessdata_dir_non_empty() {
        let d = default_tessdata_dir();
        assert!(!d.is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_default_tessdata_dir_windows_uses_appdata() {
        // Sanity check the Windows path uses backslashes and includes tesseract-rs/tessdata.
        let d = default_tessdata_dir();
        assert!(
            d.ends_with("\\tesseract-rs\\tessdata"),
            "unexpected default path on windows: {}",
            d
        );
    }

    #[tokio::test]
    async fn test_ensure_traineddata_skips_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("xyz.traineddata");
        std::fs::write(&path, b"stub").unwrap();
        // Should be a no-op (no network); language "xyz" doesn't exist upstream
        // so any actual download attempt would fail.
        ensure_traineddata(tmp.path(), "xyz").await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"stub");
    }
}
