# LiteParse

[![Crates.io version](https://img.shields.io/crates/v/liteparse.svg)](https://crates.io/crates/liteparse)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

Rust library and CLI for fast, lightweight PDF and document parsing with spatial text extraction. Runs entirely locally with zero cloud dependencies.

> LiteParse is also available for [Node.js/TypeScript](https://www.npmjs.com/package/@llamaindex/liteparse), [Python](https://pypi.org/project/liteparse/), and the [browser (WASM)](https://www.npmjs.com/package/@llamaindex/liteparse-wasm). See the [project README](https://github.com/run-llama/liteparse) for all options.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
liteparse = "2"
```

Or install the CLI:

```bash
cargo install liteparse
```

## Quick Start

```rust
use liteparse::{LiteParse, LiteParseConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parser = LiteParse::new(LiteParseConfig::default());
    let result = parser.parse("document.pdf").await?;

    println!("{}", result.text);

    for page in &result.pages {
        println!("Page {}: {} text items", page.page_num, page.text_items.len());
    }

    Ok(())
}
```

## Configuration

```rust
use liteparse::{LiteParse, LiteParseConfig, OutputFormat};

let config = LiteParseConfig {
    ocr_enabled: true,                    // Enable OCR (default: true)
    ocr_language: "eng".to_string(),      // Tesseract language code
    ocr_server_url: None,                 // HTTP OCR server URL (optional)
    tessdata_path: None,                  // Path to tessdata directory (optional)
    max_pages: 1000,                      // Max pages to parse
    target_pages: Some("1-5,10".into()),  // Specific pages (optional)
    dpi: 150.0,                           // Rendering DPI
    output_format: OutputFormat::Json,    // Json | Text | Markdown
    preserve_very_small_text: false,      // Keep tiny text
    include_text_metadata: false,         // Opt in to rich PDF text metadata
    password: None,                       // Password for protected documents
    quiet: false,                         // Suppress progress output
    ..Default::default()
};

let parser = LiteParse::new(config);
```

## Markdown Output

LiteParse can render documents directly to Markdown, including headings, tables, lists,
images, and links reconstructed from the spatial layout. Set
`output_format: OutputFormat::Markdown`; the rendered Markdown is returned on
`result.text`. Two related knobs control Markdown rendering:

- `image_mode` (`ImageMode::Placeholder` default | `Off` | `Embed`) — how raster
  images are surfaced in the output.
- `extract_links` (default `true`) — render hyperlink annotations as
  `[text](url)`; set `false` for plain anchor text.

```rust
use liteparse::config::{ImageMode, LiteParseConfig, OutputFormat};

let config = LiteParseConfig {
    output_format: OutputFormat::Markdown,
    image_mode: ImageMode::Placeholder,
    extract_links: true,
    ..Default::default()
};
let result = LiteParse::new(config).parse("document.pdf").await?;
println!("{}", result.text); // rendered Markdown
```

> Reconstruction quality varies with document complexity.

## Parsing from Bytes

```rust
use liteparse::types::PdfInput;

let pdf_bytes: Vec<u8> = std::fs::read("document.pdf")?;
let result = parser.parse_input(PdfInput::Bytes(pdf_bytes)).await?;
println!("{}", result.text);
```

## Document Complexity

Before committing to a full parse, check whether a document needs OCR or heavier
processing. `is_complex` is a cheap, text-layer-only pass that returns a
`PageComplexityStats` per page with a `needs_ocr` verdict and the signals behind it —
useful for routing documents to different pipelines, rejecting ones you can't handle, or
estimating cost.

```rust
use liteparse::types::PdfInput;

let parser = LiteParse::new(LiteParseConfig::default());
let pages = parser.is_complex(PdfInput::Path("document.pdf".into())).await?;

if pages.iter().any(|p| p.needs_ocr) {
    // Route to the OCR-enabled pipeline, inspect `p.reasons`, etc.
    for page in pages.iter().filter(|p| p.needs_ocr) {
        println!("Page {} needs OCR: {:?}", page.page_number, page.reasons);
    }
}
```

`reasons` is a `Vec<ComplexityReason>` (`Scanned`, `NoText`, `SparseText`,
`EmbeddedImages`, `Garbled`, `VectorText`); new variants may be added over time, so match
leniently.

## Custom OCR Engine

Implement the `OcrEngine` trait to plug in your own OCR backend:

```rust
use liteparse::ocr::OcrEngine;
use std::sync::Arc;

let parser = LiteParse::new(LiteParseConfig::default())
    .with_ocr_engine(Arc::new(my_engine));
```

## Features

- **`tesseract`** (default) — Built-in Tesseract OCR via `tesseract-rs`. Disable with `default-features = false` if you don't need OCR or want to use an HTTP OCR server instead.

## Supported Formats

- PDF (`.pdf`)
- Microsoft Office (`.docx`, `.xlsx`, `.pptx`, etc.) — requires LibreOffice
- OpenDocument (`.odt`, `.ods`, `.odp`) — requires LibreOffice
- Images (`.png`, `.jpg`, `.tiff`, etc.) — requires ImageMagick

## CLI

The crate also builds the `lit` CLI binary:

```bash
lit parse document.pdf
lit parse document.pdf --format json -o output.json
lit parse document.pdf --format markdown -o output.md
lit screenshot document.pdf -o ./screenshots
lit batch-parse ./input ./output
lit is-complex document.pdf
```

See `lit --help` for all options.

## License

Apache-2.0
