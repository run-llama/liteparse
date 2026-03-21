---
title: CLI Reference
description: Complete reference for all LiteParse CLI commands and options.
sidebar:
  order: 5
---

LiteParse provides the `lit` CLI with three commands: `parse`, `batch-parse`, and `screenshot`.

## `lit parse`

Parse a single document.

```
lit parse [options] <file>
```

### Arguments

| Argument | Description |
|----------|-------------|
| `file` | Path to the document file |

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output <file>` | Write output to a file instead of stdout | — |
| `--format <format>` | Output format: `json` or `text` | `text` |
| `--ocr-server-url <url>` | HTTP OCR server URL | — (uses Tesseract) |
| `--no-ocr` | Disable OCR entirely | — |
| `--ocr-language <lang>` | OCR language code | `en` |
| `--num-workers <n>` | Pages to OCR in parallel | CPU cores - 1 |
| `--max-pages <n>` | Maximum pages to parse | `10000` |
| `--target-pages <pages>` | Pages to parse (e.g., `"1-5,10"`) | — (all pages) |
| `--dpi <dpi>` | Rendering DPI | `150` |
| `--no-precise-bbox` | Disable precise bounding boxes | — |
| `--preserve-small-text` | Keep very small text | — |
| `--password <password>` | Password for encrypted/protected documents | — |
| `--config <file>` | JSON config file path | — |
| `-q, --quiet` | Suppress progress output | — |

### Examples

```bash
# Basic text parsing
lit parse report.pdf

# JSON output with bounding boxes
lit parse report.pdf --format json -o report.json

# Parse pages 1-5 only, no OCR
lit parse report.pdf --target-pages "1-5" --no-ocr

# High-DPI rendering with French OCR
lit parse report.pdf --dpi 300 --ocr-language fra

# Use an external OCR server
lit parse report.pdf --ocr-server-url http://localhost:8828/ocr

# Pipe output to another tool
lit parse report.pdf -q | wc -l
```

---

## `lit batch-parse`

Parse multiple documents in a directory.

```
lit batch-parse [options] <input-dir> <output-dir>
```

### Arguments

| Argument | Description |
|----------|-------------|
| `input-dir` | Directory containing documents to parse |
| `output-dir` | Directory for output files |

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `--format <format>` | Output format: `json` or `text` | `text` |
| `--ocr-server-url <url>` | HTTP OCR server URL | — (uses Tesseract) |
| `--no-ocr` | Disable OCR entirely | — |
| `--ocr-language <lang>` | OCR language code | `en` |
| `--num-workers <n>` | Pages to OCR in parallel | CPU cores - 1 |
| `--max-pages <n>` | Maximum pages per file | `10000` |
| `--dpi <dpi>` | Rendering DPI | `150` |
| `--no-precise-bbox` | Disable precise bounding boxes | — |
| `--recursive` | Search subdirectories | — |
| `--extension <ext>` | Only process this extension (e.g., `".pdf"`) | — (all supported) |
| `--password <password>` | Password for encrypted/protected documents (applied to all files) | — |
| `--config <file>` | JSON config file path | — |
| `-q, --quiet` | Suppress progress output | — |

### Examples

```bash
# Parse all supported files in a directory
lit batch-parse ./documents ./output

# Recursively parse only PDFs
lit batch-parse ./documents ./output --recursive --extension ".pdf"

# Batch parse with JSON output and no OCR
lit batch-parse ./documents ./output --format json --no-ocr

# Use a config file for consistent settings
lit batch-parse ./documents ./output --config liteparse.config.json
```

---

## `lit screenshot`

Generate page images from a PDF.

```
lit screenshot [options] <file>
```

### Arguments

| Argument | Description |
|----------|-------------|
| `file` | Path to the PDF file |

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output-dir <dir>` | Output directory | `./screenshots` |
| `--target-pages <pages>` | Pages to screenshot (e.g., `"1,3,5"` or `"1-5"`) | — (all pages) |
| `--dpi <dpi>` | Rendering DPI | `150` |
| `--format <format>` | Image format: `png` or `jpg` | `png` |
| `--password <password>` | Password for encrypted/protected documents | — |
| `--config <file>` | JSON config file path | — |
| `-q, --quiet` | Suppress progress output | — |

### Examples

```bash
# Screenshot all pages
lit screenshot document.pdf -o ./pages

# First 5 pages at high DPI
lit screenshot document.pdf --pages "1-5" --dpi 300 -o ./pages

# JPG format for smaller files
lit screenshot document.pdf --format jpg -o ./pages

# Specific pages only
lit screenshot document.pdf --pages "1,5,10" -o ./pages
```

---

## Global options

These options are available on all commands:

| Option | Description |
|--------|-------------|
| `-h, --help` | Show help for a command |
| `-V, --version` | Show version number |
