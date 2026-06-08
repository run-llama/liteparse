use clap::{Args, Parser, Subcommand};
use liteparse::config::{LiteParseConfig, OutputFormat};
use liteparse::conversion;
use liteparse::output::{json, text};
use liteparse::parser::LiteParse;

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
}

#[derive(Args, Debug)]
struct ParseCommand {
    file: String,
    #[arg(short, long)]
    output: Option<String>,
    #[arg(long, default_value = "text")]
    format: String,
    #[arg(long)]
    no_ocr: bool,
    #[arg(long, default_value = "eng")]
    ocr_language: String,
    #[arg(long, default_value = None)]
    ocr_server_url: Option<String>,
    #[arg(long)]
    tessdata_path: Option<String>,
    #[arg(long, default_value = "1000")]
    max_pages: usize,
    #[arg(long)]
    target_pages: Option<String>,
    #[arg(long, default_value = "150")]
    dpi: f32,
    #[arg(long)]
    preserve_small_text: bool,
    #[arg(long)]
    password: Option<String>,
    #[arg(short, long)]
    quiet: bool,
    #[arg(long)]
    num_workers: Option<usize>,
    #[arg(long)]
    inline_images: bool,
}

#[derive(Args, Debug)]
struct ScreenshotCommand {
    file: String,
    #[arg(short, long, default_value = "./screenshots")]
    output_dir: String,
    #[arg(long)]
    target_pages: Option<String>,
    #[arg(long, default_value = "150")]
    dpi: f32,
    #[arg(long)]
    password: Option<String>,
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Args, Debug)]
struct BatchParseCommand {
    input_dir: String,
    output_dir: String,
    #[arg(long, default_value = "text")]
    format: String,
    #[arg(long)]
    no_ocr: bool,
    #[arg(long, default_value = "eng")]
    ocr_language: String,
    #[arg(long, default_value = None)]
    ocr_server_url: Option<String>,
    #[arg(long)]
    tessdata_path: Option<String>,
    #[arg(long, default_value = "1000")]
    max_pages: usize,
    #[arg(long, default_value = "150")]
    dpi: f32,
    #[arg(long)]
    recursive: bool,
    #[arg(long)]
    extension: Option<String>,
    #[arg(long)]
    password: Option<String>,
    #[arg(short, long)]
    quiet: bool,
    #[arg(long)]
    num_workers: Option<usize>,
    #[arg(long)]
    inline_images: bool,
}

fn parse_output_format(s: &str) -> Result<OutputFormat, String> {
    match s.to_lowercase().as_str() {
        "json" => Ok(OutputFormat::Json),
        "text" => Ok(OutputFormat::Text),
        _ => Err(format!("unknown format '{}', expected 'json' or 'text'", s)),
    }
}

/// Run the CLI with the given args (typically from sys.argv).
pub fn run_cli(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse_from(args);
    let rt = tokio::runtime::Runtime::new()?;

    match cli.command {
        Commands::Parse(cmd) => {
            let format = parse_output_format(&cmd.format)?;
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
                inline_images: cmd.inline_images,
                ..Default::default()
            };
            if let Some(n) = cmd.num_workers {
                config.num_workers = n;
            }
            let lp = LiteParse::new(config);
            let result = rt.block_on(lp.parse(&cmd.file))?;
            let formatted = match lp.config().output_format {
                OutputFormat::Json => json::format_json(&result.pages)?,
                OutputFormat::Text => text::format_text(&result.pages),
            };
            match cmd.output {
                Some(path) => {
                    std::fs::write(&path, &formatted)?;
                    if !cmd.quiet {
                        eprintln!("[liteparse] wrote output to {}", path);
                    }
                }
                None => println!("{}", formatted),
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
            let results = rt.block_on(lp.screenshot(&cmd.file, target_pages))?;

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
                inline_images: cmd.inline_images,
                ..Default::default()
            };
            if let Some(n) = cmd.num_workers {
                config.num_workers = n;
            }

            let lp = LiteParse::new(config);
            let out_ext = if format == OutputFormat::Json {
                "json"
            } else {
                "txt"
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
                let t0 = std::time::Instant::now();
                let rel = file_path.strip_prefix(&cmd.input_dir).unwrap_or(file_path);
                let out_path = std::path::Path::new(&cmd.output_dir)
                    .join(rel)
                    .with_extension(out_ext);

                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                match rt.block_on(lp.parse(file_path)) {
                    Ok(result) => {
                        let fmt_result: Result<String, Box<dyn std::error::Error>> =
                            match lp.config().output_format {
                                OutputFormat::Json => {
                                    json::format_json(&result.pages).map_err(|e| e.into())
                                }
                                OutputFormat::Text => Ok(text::format_text(&result.pages)),
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
    }

    Ok(())
}

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
