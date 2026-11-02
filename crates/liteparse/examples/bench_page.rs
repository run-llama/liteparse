//! Single-pass benchmark worker. OCR replay is a separate post-OCR experiment profile.
use clap::{Parser, ValueEnum};
use liteparse::LiteParse;
use liteparse::config::{ImageMode, LiteParseConfig, OutputFormat};
use liteparse::ocr::{OcrEngine, OcrOptions, OcrResult};
use serde::{Deserialize, Serialize};
use std::{error::Error, path::PathBuf, pin::Pin, sync::Arc};

type OcrError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum Mode {
    Off,
    Record,
    Replay,
}

#[derive(Parser)]
struct Args {
    file: String,
    #[arg(long)]
    page: usize,
    #[arg(long, value_enum, default_value = "off")]
    ocr_mode: Mode,
    #[arg(long)]
    ocr_cache: Option<PathBuf>,
    /// Immutable engine/model/settings identifier, supplied by the operator.
    #[arg(long)]
    ocr_id: Option<String>,
    #[arg(long)]
    ocr_server_url: Option<String>,
    #[arg(long, default_value = "eng")]
    language: String,
    #[arg(long, default_value_t = 150.0)]
    dpi: f32,
}

#[derive(Serialize, Deserialize, PartialEq)]
struct Profile {
    version: u32,
    engine_id: String,
    grayscale: bool,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    request_hash: String,
    results: Vec<OcrResult>,
}

struct CachedOcr {
    root: PathBuf,
    profile: Profile,
    inner: Option<Arc<dyn OcrEngine>>,
}

impl CachedOcr {
    fn key(&self, image: &[u8], width: u32, height: u32, options: &OcrOptions) -> String {
        let mut hash = blake3::Hasher::new();
        hash.update(&serde_json::to_vec(&self.profile).unwrap());
        hash.update(&width.to_le_bytes());
        hash.update(&height.to_le_bytes());
        hash.update(&options.dpi.to_bits().to_le_bytes());
        hash.update(&(options.language.len() as u64).to_le_bytes());
        hash.update(options.language.as_bytes());
        hash.update(image);
        hash.finalize().to_hex().to_string()
    }
}

impl OcrEngine for CachedOcr {
    fn name(&self) -> &str {
        "benchmark-ocr-cache"
    }
    fn prefers_grayscale(&self) -> bool {
        self.profile.grayscale
    }
    fn recognize<'a, 'b: 'a, 'c: 'a>(
        &'a self,
        image: &'c [u8],
        width: u32,
        height: u32,
        options: &'b OcrOptions,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<OcrResult>, OcrError>> + Send + '_>> {
        Box::pin(async move {
            let key = self.key(image, width, height, options);
            let path = self.root.join(format!("{key}.json"));
            if path.exists() {
                let entry: Entry = serde_json::from_slice(&std::fs::read(path)?)?;
                if entry.request_hash != key {
                    return Err("OCR cache key mismatch".into());
                }
                return Ok(entry.results);
            }
            let engine = self.inner.as_ref().ok_or_else(|| -> OcrError {
                format!("OCR replay cache miss: {key}; prime a new shared snapshot before timing")
                    .into()
            })?;
            let results = engine.recognize(image, width, height, options).await?;
            let entry = Entry {
                request_hash: key,
                results: results.clone(),
            };
            let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
            serde_json::to_writer(temp.as_file_mut(), &entry)?;
            temp.persist_noclobber(path)?;
            Ok(results)
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), OcrError> {
    let args = Args::parse();
    if args.page == 0 || !args.dpi.is_finite() || args.dpi <= 0.0 {
        return Err("page and DPI must be positive".into());
    }
    if args.ocr_mode != Mode::Record && args.ocr_server_url.is_some() {
        return Err("upstream OCR is only allowed in record mode".into());
    }
    let config = LiteParseConfig {
        output_format: OutputFormat::Markdown,
        extract_blocks: true,
        extract_links: false,
        image_mode: ImageMode::Off,
        ocr_enabled: args.ocr_mode != Mode::Off,
        ocr_failure_fatal: true,
        ocr_server_url: args.ocr_server_url,
        ocr_language: args.language,
        dpi: args.dpi,
        target_pages: Some(args.page.to_string()),
        max_pages: args.page,
        num_workers: 1,
        quiet: true,
        ..Default::default()
    };
    let mut parser = LiteParse::new(config);
    if args.ocr_mode != Mode::Off {
        let root = args.ocr_cache.ok_or("--ocr-cache is required")?;
        let engine_id = args.ocr_id.ok_or("--ocr-id is required")?;
        let profile_path = root.join("profile.json");
        let inner = if args.ocr_mode == Mode::Record {
            parser.ocr_engine()?
        } else {
            None
        };
        let profile = if let Some(ref engine) = inner {
            std::fs::create_dir_all(&root)?;
            let profile = Profile {
                version: 1,
                engine_id,
                grayscale: engine.prefers_grayscale(),
            };
            if profile_path.exists() {
                let existing: Profile = serde_json::from_slice(&std::fs::read(&profile_path)?)?;
                if existing != profile {
                    return Err("OCR cache belongs to another engine/profile".into());
                }
            } else {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(profile_path)?;
                serde_json::to_writer(&mut file, &profile)?;
            }
            profile
        } else {
            let profile: Profile = serde_json::from_slice(&std::fs::read(profile_path)?)?;
            if profile.version != 1 || profile.engine_id != engine_id {
                return Err("OCR replay profile mismatch".into());
            }
            profile
        };
        parser = parser.with_ocr_engine(Arc::new(CachedOcr {
            root,
            profile,
            inner,
        }));
    }
    let result = parser.parse(&args.file).await?;
    if !result.page_errors.is_empty()
        || result.pages.len() != 1
        || result.pages[0].page_number != args.page
    {
        return Err("requested page was not parsed successfully".into());
    }
    let page = &result.pages[0];
    // Markdown and layout come from the exact same parse, with no second CLI call.
    serde_json::to_writer(
        std::io::stdout().lock(),
        &serde_json::json!({
            "page": page.page_number,
            "total_pages": result.total_pages,
            "width": page.page_width,
            "height": page.page_height,
            "markdown": page.markdown,
            "blocks": page.blocks,
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replay_keys_include_render_and_engine_inputs_and_misses_fail() {
        let directory = tempfile::tempdir().unwrap();
        let cache = CachedOcr {
            root: directory.path().into(),
            profile: Profile {
                version: 1,
                engine_id: "fixed-model".into(),
                grayscale: true,
            },
            inner: None,
        };
        let mut options = OcrOptions {
            language: "eng".into(),
            dpi: 150.0,
        };
        let key = cache.key(&[1, 2], 2, 1, &options);
        assert_ne!(key, cache.key(&[1, 3], 2, 1, &options));
        assert_ne!(key, cache.key(&[1, 2], 1, 2, &options));
        options.dpi = 300.0;
        assert_ne!(key, cache.key(&[1, 2], 2, 1, &options));
        options.dpi = 150.0;
        options.language = "fra".into();
        assert_ne!(key, cache.key(&[1, 2], 2, 1, &options));
        options.language = "eng".into();
        assert!(cache.recognize(&[1, 2], 2, 1, &options).await.is_err());
        std::fs::write(
            directory.path().join(format!("{key}.json")),
            serde_json::to_vec(&Entry {
                request_hash: key.clone(),
                results: vec![OcrResult {
                    text: "cached".into(),
                    bbox: [0., 0., 2., 1.],
                    confidence: 1.,
                    polygon: None,
                }],
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            cache.recognize(&[1, 2], 2, 1, &options).await.unwrap()[0].text,
            "cached"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
