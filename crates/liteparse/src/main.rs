use clap::{Args, Parser, Subcommand};
use liteparse::config::{LiteParseConfig, OutputFormat};
use liteparse::conversion;
use liteparse::extract;
use liteparse::output::{json, text};
use liteparse::parser::LiteParse;
use liteparse::render;
use liteparse::types::PdfInput;

#[derive(Parser, Debug)]
#[command(
    name = "lit",
    version,
    about = "OSS document parsing tool (supports PDF, DOCX, XLSX, images, and more)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Parse a document file (PDF, DOCX, XLSX, PPTX, images, etc.)
    Parse(ParseCommand),
    /// Generate screenshots of document pages (PDF, DOCX, XLSX, images, etc.)
    Screenshot(ScreenshotCommand),
    /// Parse multiple documents in batch mode
    BatchParse(BatchParseCommand),
    /// Check if a document is "complex" enough to require OCR or other advanced parsing
    IsComplex(IsComplexCommand),
    /// Extract raw text items from a PDF file (no grid projection) [dev tool]
    #[command(hide = true)]
    Extract(ExtractCommand),
    /// Extract embedded image bounding boxes from a page [dev tool]
    #[command(hide = true)]
    ImageBounds(ExtractCommand),
}

#[derive(Args, Debug)]
struct ParseCommand {
    /// Input file path
    file: String,

    /// Output file path
    #[arg(short, long)]
    output: Option<String>,

    /// Output format: json, text, or markdown
    #[arg(long, default_value = "text")]
    format: String,

    /// Disable OCR
    #[arg(long)]
    no_ocr: bool,

    /// OCR language (Tesseract format, e.g. "eng", "fra", "deu")
    #[arg(long, default_value = "eng")]
    ocr_language: String,

    /// HTTP OCR server URL (uses Tesseract if not provided)
    #[arg(long, default_value = None)]
    ocr_server_url: Option<String>,

    /// Extra header for OCR server requests, "Name: Value" (repeatable).
    /// e.g. --ocr-server-header "Authorization: Bearer <token>"
    #[arg(long = "ocr-server-header", value_parser = parse_header)]
    ocr_server_headers: Vec<(String, String)>,

    /// Path to tessdata directory (overrides TESSDATA_PREFIX env var)
    #[arg(long)]
    tessdata_path: Option<String>,

    /// Max pages to parse
    #[arg(long, default_value = "1000")]
    max_pages: usize,

    /// Target pages (e.g., "1-5,10,15-20")
    #[arg(long)]
    target_pages: Option<String>,

    /// DPI for rendering (default: 150)
    #[arg(long, default_value = "150")]
    dpi: f32,

    /// Preserve very small text
    #[arg(long)]
    preserve_small_text: bool,

    /// Password for encrypted/protected documents
    #[arg(long)]
    password: Option<String>,

    /// Suppress progress output
    #[arg(short, long)]
    quiet: bool,

    /// Number of concurrent OCR workers (default: CPU cores - 1)
    #[arg(long)]
    num_workers: Option<usize>,

    /// How to surface raster images in markdown output:
    /// `off` strips them, `placeholder` (default) emits `![](image_pN_K.png)`
    /// references in reading order, `embed` extracts each image's PNG bytes
    /// and writes them next to the markdown output when `--image-output-dir`
    /// is set.
    #[arg(long, default_value = "placeholder")]
    image_mode: String,

    /// Directory to write embedded images to when `--image-mode embed` is
    /// set. Each image is written as `image_{id}.png` to match the
    /// references in the markdown output. Has no effect for other image
    /// modes. Created if missing.
    #[arg(long)]
    image_output_dir: Option<String>,

    /// Disable hyperlink extraction. By default, URI link annotations are
    /// rendered as `[text](url)` in markdown output. Pass this to emit the
    /// anchor text as plain text instead (e.g. for plain-text benchmark
    /// parity, where ground truth uses no link syntax).
    #[arg(long)]
    no_links: bool,

    /// Include rich PDF text metadata in text items and JSON output.
    #[arg(long)]
    include_text_metadata: bool,
}

#[derive(Args, Debug)]
struct ScreenshotCommand {
    /// Input document path (PDF, DOCX, XLSX, images, etc.)
    file: String,

    /// Output directory for screenshots
    #[arg(short, long, default_value = "./screenshots")]
    output_dir: String,

    /// Target pages (e.g., "1,3,5" or "1-5"). Defaults to all pages.
    #[arg(long)]
    target_pages: Option<String>,

    /// DPI for rendering
    #[arg(long, default_value = "150")]
    dpi: f32,

    /// Password for encrypted/protected documents
    #[arg(long)]
    password: Option<String>,

    /// Suppress progress output
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Args, Debug)]
struct BatchParseCommand {
    /// Input directory
    input_dir: String,

    /// Output directory
    output_dir: String,

    /// Output format: json, text, or markdown
    #[arg(long, default_value = "text")]
    format: String,

    /// Disable OCR
    #[arg(long)]
    no_ocr: bool,

    /// OCR language (Tesseract format, e.g. "eng", "fra", "deu")
    #[arg(long, default_value = "eng")]
    ocr_language: String,

    /// HTTP OCR server URL (uses Tesseract if not provided)
    #[arg(long, default_value = None)]
    ocr_server_url: Option<String>,

    /// Extra header for OCR server requests, "Name: Value" (repeatable).
    /// e.g. --ocr-server-header "Authorization: Bearer <token>"
    #[arg(long = "ocr-server-header", value_parser = parse_header)]
    ocr_server_headers: Vec<(String, String)>,

    /// Path to tessdata directory (overrides TESSDATA_PREFIX env var)
    #[arg(long)]
    tessdata_path: Option<String>,

    /// Max pages to parse per file
    #[arg(long, default_value = "1000")]
    max_pages: usize,

    /// DPI for rendering
    #[arg(long, default_value = "150")]
    dpi: f32,

    /// Recursively search input directory
    #[arg(long)]
    recursive: bool,

    /// Only process files with this extension (e.g., ".pdf")
    #[arg(long)]
    extension: Option<String>,

    /// Password for encrypted/protected documents
    #[arg(long)]
    password: Option<String>,

    /// Suppress progress output
    #[arg(short, long)]
    quiet: bool,

    /// Number of concurrent OCR workers (default: CPU cores - 1)
    #[arg(long)]
    num_workers: Option<usize>,

    /// Include rich PDF text metadata in text items and JSON output.
    #[arg(long)]
    include_text_metadata: bool,
}

#[derive(Args, Debug)]
struct ExtractCommand {
    /// Input PDF file path
    #[arg(long)]
    pdf_path: String,

    /// Target page number (1-based)
    #[arg(long)]
    page_num: Option<u32>,
}

#[derive(Args, Debug)]
struct IsComplexCommand {
    /// Input file path
    file: String,

    /// Emit dense, whitespace-free JSON instead of pretty-printed (still valid
    /// for `jq` and friends).
    #[arg(long)]
    compact: bool,

    /// Max pages to parse
    #[arg(long, default_value = "1000")]
    max_pages: usize,

    /// Target pages (e.g., "1-5,10,15-20")
    #[arg(long)]
    target_pages: Option<String>,

    /// Password for encrypted/protected documents
    #[arg(long)]
    password: Option<String>,

    /// Suppress progress output
    #[arg(short, long)]
    quiet: bool,
}

fn parse_output_format(s: &str) -> Result<OutputFormat, String> {
    match s.to_lowercase().as_str() {
        "json" => Ok(OutputFormat::Json),
        "text" => Ok(OutputFormat::Text),
        "markdown" | "md" => Ok(OutputFormat::Markdown),
        _ => Err(format!(
            "unknown format '{}', expected 'json', 'text', or 'markdown'",
            s
        )),
    }
}

/// Parse a `Name: Value` header string into a `(name, value)` pair.
fn parse_header(s: &str) -> Result<(String, String), String> {
    let (name, value) = s
        .split_once(':')
        .ok_or_else(|| format!("invalid header '{}', expected 'Name: Value'", s))?;
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("invalid header '{}', empty header name", s));
    }
    Ok((name.to_string(), value.trim().to_string()))
}

fn parse_image_mode(s: &str) -> Result<liteparse::config::ImageMode, String> {
    use liteparse::config::ImageMode;
    match s.to_lowercase().as_str() {
        "off" | "none" => Ok(ImageMode::Off),
        "placeholder" => Ok(ImageMode::Placeholder),
        "embed" => Ok(ImageMode::Embed),
        _ => Err(format!(
            "unknown image-mode '{}', expected 'off', 'placeholder', or 'embed'",
            s
        )),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Parse(cmd) => {
            let format = parse_output_format(&cmd.format)?;
            let image_mode = parse_image_mode(&cmd.image_mode)?;

            let mut config = LiteParseConfig {
                ocr_language: cmd.ocr_language,
                ocr_enabled: !cmd.no_ocr,
                tessdata_path: cmd.tessdata_path,
                max_pages: cmd.max_pages,
                target_pages: cmd.target_pages,
                dpi: cmd.dpi,
                output_format: format,
                preserve_very_small_text: cmd.preserve_small_text,
                password: cmd.password,
                quiet: cmd.quiet,
                ocr_server_url: cmd.ocr_server_url,
                ocr_server_headers: cmd.ocr_server_headers,
                image_mode,
                extract_links: !cmd.no_links,
                include_text_metadata: cmd.include_text_metadata,
                ..Default::default()
            };
            if let Some(n) = cmd.num_workers {
                config.num_workers = n;
            }

            let lp = LiteParse::new(config);
            let result = lp.parse(&cmd.file).await?;
            let formatted = match lp.config().output_format {
                OutputFormat::Json => json::format_json_with_text_metadata(
                    &result.pages,
                    lp.config().include_text_metadata,
                )?,
                OutputFormat::Text => text::format_text(&result.pages),
                OutputFormat::Markdown => result.text.clone(),
            };
            if let Some(dir) = cmd.image_output_dir.as_deref()
                && !result.images.is_empty()
            {
                std::fs::create_dir_all(dir)?;
                for img in &result.images {
                    let path = format!("{}/image_{}.{}", dir, img.id, img.format);
                    std::fs::write(&path, &img.bytes)?;
                }
                if !cmd.quiet {
                    eprintln!(
                        "[liteparse] wrote {} image(s) to {}",
                        result.images.len(),
                        dir
                    );
                }
            }

            match cmd.output {
                Some(path) => {
                    std::fs::write(&path, &formatted)?;
                    if !cmd.quiet {
                        eprintln!("[liteparse] wrote output to {}", path);
                    }
                }
                None => {
                    println!("{}", formatted);
                }
            }
        }

        Commands::Screenshot(cmd) => {
            let target_pages = cmd
                .target_pages
                .as_ref()
                .map(|s| liteparse::config::parse_target_pages(s))
                .transpose()
                .map_err(|e| format!("invalid --target-pages: {}", e))?;

            std::fs::create_dir_all(&cmd.output_dir)?;

            let config = LiteParseConfig {
                target_pages: cmd.target_pages.clone(),
                dpi: cmd.dpi,
                password: cmd.password.clone(),
                quiet: cmd.quiet,
                ..Default::default()
            };
            let lp = LiteParse::new(config);
            let results = lp.screenshot(&cmd.file, target_pages).await?;

            for result in results {
                let output_path = format!("{}/page_{}.png", cmd.output_dir, result.page_num);
                std::fs::write(&output_path, &result.image_bytes)?;

                if !cmd.quiet {
                    eprintln!(
                        "[liteparse] screenshot page {} → {}",
                        result.page_num, output_path
                    );
                }
            }
        }

        Commands::BatchParse(cmd) => {
            let format = parse_output_format(&cmd.format)?;
            let ext_filter = cmd.extension.as_ref().map(|e| {
                let e = e.to_lowercase();
                if e.starts_with('.') {
                    e
                } else {
                    format!(".{}", e)
                }
            });

            let mut config = LiteParseConfig {
                ocr_language: cmd.ocr_language,
                ocr_enabled: !cmd.no_ocr,
                tessdata_path: cmd.tessdata_path,
                max_pages: cmd.max_pages,
                target_pages: None,
                dpi: cmd.dpi,
                output_format: format.clone(),
                preserve_very_small_text: false,
                password: cmd.password,
                quiet: cmd.quiet,
                ocr_server_url: cmd.ocr_server_url,
                ocr_server_headers: cmd.ocr_server_headers,
                include_text_metadata: cmd.include_text_metadata,
                ..Default::default()
            };
            if let Some(n) = cmd.num_workers {
                config.num_workers = n;
            }

            let lp = LiteParse::new(config);
            let out_ext = match format {
                OutputFormat::Json => "json",
                OutputFormat::Markdown => "md",
                OutputFormat::Text => "txt",
            };

            std::fs::create_dir_all(&cmd.output_dir)?;

            let files = collect_files(&cmd.input_dir, cmd.recursive, ext_filter.as_deref())?;

            if files.is_empty() {
                eprintln!("[liteparse] no matching files found in {}", cmd.input_dir);
                return Ok(());
            }

            if !cmd.quiet {
                eprintln!("[liteparse] found {} files to process", files.len());
            }

            let mut success = 0usize;
            let mut errors = 0usize;

            for file_path in &files {
                let t0 = web_time::Instant::now();

                let out_path =
                    batch_output_path(file_path, &cmd.input_dir, &cmd.output_dir, out_ext);

                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                match lp.parse(file_path).await {
                    Ok(result) => {
                        let fmt_result: Result<String, Box<dyn std::error::Error>> =
                            match lp.config().output_format {
                                OutputFormat::Json => json::format_json_with_text_metadata(
                                    &result.pages,
                                    lp.config().include_text_metadata,
                                )
                                .map_err(|e| e.into()),
                                OutputFormat::Text => Ok(text::format_text(&result.pages)),
                                OutputFormat::Markdown => Ok(result.text.clone()),
                            };
                        match fmt_result {
                            Ok(formatted) => {
                                std::fs::write(&out_path, &formatted)?;
                                success += 1;
                                if !cmd.quiet {
                                    let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
                                    eprintln!(
                                        "[liteparse] {} → {} ({:.1}ms)",
                                        file_path,
                                        out_path.display(),
                                        elapsed
                                    );
                                }
                            }
                            Err(e) => {
                                eprintln!("[liteparse] error formatting {}: {}", file_path, e);
                                errors += 1;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[liteparse] error parsing {}: {}", file_path, e);
                        errors += 1;
                    }
                }
            }

            eprintln!(
                "[liteparse] batch complete: {} succeeded, {} failed",
                success, errors
            );

            if errors > 0 {
                std::process::exit(1);
            }
        }

        Commands::Extract(cmd) => {
            extract::extract(&cmd.pdf_path, cmd.page_num)?;
        }

        Commands::ImageBounds(cmd) => {
            render::image_bounds(&cmd.pdf_path, cmd.page_num)?;
        }

        Commands::IsComplex(cmd) => {
            let config = LiteParseConfig {
                max_pages: cmd.max_pages,
                target_pages: cmd.target_pages,
                password: cmd.password,
                quiet: cmd.quiet,
                ..Default::default()
            };
            let lp = LiteParse::new(config);
            let is_complex = lp.is_complex(PdfInput::Path(cmd.file)).await?;

            let complex_pages = is_complex.iter().filter(|c| c.needs_ocr).count();

            // Always emit JSON on stdout so the command composes with `jq` and
            // friends without a flag. Pretty by default for human readability;
            // `--compact` drops the whitespace. Both parse identically.
            let json = if cmd.compact {
                serde_json::to_string(&is_complex)?
            } else {
                serde_json::to_string_pretty(&is_complex)?
            };
            println!("{}", json);

            // The human-readable verdict goes to stderr so it never pollutes the
            // JSON on stdout. The exit code below carries the same signal for
            // scripts that don't want to read either stream.
            if !cmd.quiet {
                let verdict = if complex_pages > 0 {
                    "COMPLEX"
                } else {
                    "SIMPLE"
                };
                eprintln!(
                    "{} — {}/{} page(s) need OCR",
                    verdict,
                    complex_pages,
                    is_complex.len()
                );
            }

            // Exit non-zero when any page needs OCR, so the command is usable as
            // a shell predicate: exit 0 (simple) → `&& parse --no-ocr` is safe.
            if complex_pages > 0 {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// Collect files from a directory, optionally recursively, with an optional extension filter.
fn collect_files(
    dir: &str,
    recursive: bool,
    ext_filter: Option<&str>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    collect_files_inner(std::path::Path::new(dir), recursive, ext_filter, &mut files)?;
    files.sort();
    Ok(files)
}

fn batch_output_path(
    file_path: &str,
    input_dir: &str,
    output_dir: &str,
    out_ext: &str,
) -> std::path::PathBuf {
    let file_path = std::path::Path::new(file_path);
    let rel = file_path
        .strip_prefix(std::path::Path::new(input_dir))
        .unwrap_or(file_path);

    std::path::Path::new(output_dir)
        .join(rel)
        .with_extension(out_ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn batch_output_path_preserves_output_dir_without_trailing_slash() {
        let out_path = batch_output_path("docs/report.pdf", "docs", "out", "txt");

        assert_eq!(out_path, Path::new("out/report.txt"));
    }

    #[test]
    fn batch_output_path_mirrors_nested_files_without_trailing_slash() {
        let out_path = batch_output_path("docs/nested/report.pdf", "docs", "out", "md");

        assert_eq!(out_path, Path::new("out/nested/report.md"));
    }

    #[test]
    fn include_text_metadata_flag_is_available_for_parse_and_batch() {
        let cli = Cli::try_parse_from(["lit", "parse", "document.pdf", "--include-text-metadata"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Parse(ParseCommand {
                include_text_metadata: true,
                ..
            })
        ));

        let cli = Cli::try_parse_from([
            "lit",
            "batch-parse",
            "input",
            "output",
            "--include-text-metadata",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::BatchParse(BatchParseCommand {
                include_text_metadata: true,
                ..
            })
        ));
    }
}

fn collect_files_inner(
    dir: &std::path::Path,
    recursive: bool,
    ext_filter: Option<&str>,
    files: &mut Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if recursive {
                collect_files_inner(&path, recursive, ext_filter, files)?;
            }
            continue;
        }

        let path_str = path.to_string_lossy().to_string();

        if let Some(filter) = ext_filter {
            if !path_str.to_lowercase().ends_with(filter) {
                continue;
            }
        } else if !conversion::is_supported_extension(&path_str) {
            continue;
        }

        files.push(path_str);
    }
    Ok(())
}
