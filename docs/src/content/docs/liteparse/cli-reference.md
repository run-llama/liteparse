---
title: CLI Reference
description: Complete reference for all LiteParse CLI commands and options.
sidebar:
  order: 5
---

LiteParse provides the `lit` CLI with four commands: `parse`, `batch-parse`, `screenshot`, and `is-complex`. The CLI is the same whether installed via `npm`, `pip`, or built from Rust source.

## `lit parse`

Parse a single document.

```
lit parse [options] <file>
```

### Arguments

| Argument | Description |
|----------|-------------|
| `file` | Path to the document file, or `-` to read from stdin |

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output <file>` | Write output to a file instead of stdout | — |
| `--format <format>` | Output format: `json`, `text`, or `markdown` | `text` |
| `--image-mode <mode>` | Markdown image handling: `off`, `placeholder`, or `embed` | `placeholder` |
| `--image-output-dir <dir>` | Directory to write extracted images to. Requires `--extract-images` (or `--image-mode embed`, which implies it) | — |
| `--no-links` | Emit link anchor text as plain text (no `[text](url)`) in markdown | — |
| `--keep-headers-footers` | Keep running header/footer chrome in markdown instead of stripping it | — |
| `--no-ocr` | Disable OCR entirely | — |
| `--ocr-language <lang>` | OCR language code (Tesseract format) | `eng` |
| `--ocr-backend <backend>` | `auto`, `tesseract`, `http`, or `macos-vision` (macOS 10.15+) | `auto` |
| `--mac-ocr-language <tag>` | Vision language tag, e.g. `en-US`. Repeatable or comma-separated, in priority order | Vision automatic/default languages |
| `--mac-ocr-recognition <level>` | Vision recognition level: `accurate` or `fast` | `accurate` |
| `--ocr-server-url <url>` | HTTP OCR server URL | — (uses Tesseract) |
| `--ocr-server-header <header>` | Extra HTTP header for OCR server requests, as `"Name: Value"`. Repeatable | — |
| `--tessdata-path <path>` | Path to tessdata directory | — (uses `TESSDATA_PREFIX` env var) |
| `--num-workers <n>` | Pages to OCR in parallel | CPU cores - 1 |
| `--max-pages <n>` | Maximum pages to parse | `1000` |
| `--target-pages <pages>` | Pages to parse (e.g., `"1-5,10"`) | — (all pages) |
| `--dpi <dpi>` | Rendering DPI | `150` |
| `--preserve-small-text` | Keep very small text | — |
| `--password <password>` | Password for encrypted/protected documents | — |
| `-q, --quiet` | Suppress progress output | — |

### Extraction options

By default, JSON output contains only text, geometry, and page metadata. Each option below is **off by default** and adds a new key to the JSON output. Enable only what you need — every one of them costs parse time and output size.

| Option | Adds to JSON | Description |
|--------|--------------|-------------|
| `--extract-images` | `images[]`, `image_error_count` | Extract embedded raster image bytes and metadata. Pair with `--image-output-dir` to write files to disk |
| `--extract-vector-graphics` | `pages[].vector_graphics` | Vector shapes and lines (`{ shapes, lines }`) with bounding boxes, stroke/fill colors |
| `--extract-annotations` | `pages[].annotations` | PDF annotations: subtype, contents, timestamps, rects, link URIs |
| `--extract-form-fields` | `pages[].form_fields` | AcroForm widgets: type, name, value, flags, options, rect |
| `--extract-structure-tree` | `pages[].structure_tree` | Tagged-PDF logical structure tree (`{ roots }`), including per-element attributes |
| `--extract-blocks` | `pages[].blocks` | Classified layout blocks in reading order (headings, paragraphs, list items, tables with per-cell boxes, code, rules, figures), each with a bounding box |
| `--extract-content-bounds` | `pages[].content_bounds` | Bounding box of actual content on the page, as a `Rect` |
| `--extract-xfa-packets` | `xfa_packets[]` (document level) | Raw XFA packets from XFA-based forms |
| `--extract-text-metadata` | extra keys on each `text_items[]` entry | Rich per-item typography: `font_height`, `font_ascent`, `font_descent`, `font_weight`, `text_width`, `font_is_buggy`, `mcid`, `fill_color`, `stroke_color`, plus `rotation` |
| `--complexity` | `pages[].complexity` | Per-page complexity signals, including the nested `layout` object. See the [Document Complexity guide](/liteparse/guides/complexity/) |

### Examples

```bash
# Basic text parsing
lit parse report.pdf

# JSON output with bounding boxes
lit parse report.pdf --format json -o report.json

# Markdown output (headings, tables, lists, images, links)
lit parse report.pdf --format markdown -o report.md

# Markdown with extracted images written to disk
lit parse report.pdf --format markdown --extract-images --image-output-dir ./images

# Pull form fields and annotations out of a filled-in PDF
lit parse form.pdf --format json --extract-form-fields --extract-annotations

# Rich typography metadata on every text item
lit parse report.pdf --format json --extract-text-metadata

# Inline complexity signals (including layout) alongside the parse
lit parse report.pdf --format json --complexity

# Authenticated OCR server
lit parse scan.pdf --ocr-server-url https://ocr.internal/ocr \
  --ocr-server-header "Authorization: Bearer $TOKEN"

# Parse pages 1-5 only, no OCR
lit parse report.pdf --target-pages "1-5" --no-ocr

# High-DPI rendering with French OCR
lit parse report.pdf --dpi 300 --ocr-language fra

# Use an external OCR server
lit parse report.pdf --ocr-server-url http://localhost:8828/ocr

# Pipe output to another tool
lit parse report.pdf -q | wc -l

# Parse a remote file via stdin
curl -sL https://example.com/report.pdf | lit parse --no-ocr -
```

---

## `lit batch-parse`

Parse multiple documents in a directory. The OCR backend and Apple Vision options
from `parse` are also available for `batch-parse`.

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
| `--format <format>` | Output format: `json`, `text`, or `markdown` | `text` |
| `--no-ocr` | Disable OCR entirely | — |
| `--ocr-language <lang>` | OCR language code | `eng` |
| `--ocr-server-url <url>` | HTTP OCR server URL | — (uses Tesseract) |
| `--ocr-server-header <header>` | Extra HTTP header for OCR server requests, as `"Name: Value"`. Repeatable | — |
| `--tessdata-path <path>` | Path to tessdata directory | — |
| `--num-workers <n>` | Pages to OCR in parallel | CPU cores - 1 |
| `--max-pages <n>` | Maximum pages per file | `1000` |
| `--dpi <dpi>` | Rendering DPI | `150` |
| `--recursive` | Search subdirectories | — |
| `--extension <ext>` | Only process this extension (e.g., `".pdf"`) | — (all supported) |
| `--password <password>` | Password for encrypted/protected documents (applied to all files) | — |
| `-q, --quiet` | Suppress progress output | — |

`batch-parse` accepts the same [extraction options](#extraction-options) as `lit parse`: `--extract-images`, `--extract-vector-graphics`, `--extract-annotations`, `--extract-form-fields`, `--extract-structure-tree`, `--extract-blocks`, `--extract-content-bounds`, `--extract-xfa-packets`, `--extract-text-metadata`, and `--complexity`.

It does **not** accept `--target-pages`, `--preserve-small-text`, `--image-mode`, `--image-output-dir`, `--no-links`, or `--keep-headers-footers`. Use `lit parse` per file when you need those.

### Examples

```bash
# Parse all supported files in a directory
lit batch-parse ./documents ./output

# Recursively parse only PDFs
lit batch-parse ./documents ./output --recursive --extension ".pdf"

# Batch parse with JSON output and no OCR
lit batch-parse ./documents ./output --format json --no-ocr
```

---

## `lit screenshot`

Generate page images from a document (PDF, DOCX, XLSX, images, etc.).

```
lit screenshot [options] <file>
```

### Arguments

| Argument | Description |
|----------|-------------|
| `file` | Path to the document file |

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output-dir <dir>` | Output directory | `./screenshots` |
| `--target-pages <pages>` | Pages to screenshot (e.g., `"1,3,5"` or `"1-5"`) | — (all pages) |
| `--dpi <dpi>` | Rendering DPI | `150` |
| `--password <password>` | Password for encrypted/protected documents | — |
| `-q, --quiet` | Suppress progress output | — |

### Examples

```bash
# Screenshot all pages of a PDF
lit screenshot document.pdf -o ./pages

# Screenshot a Word document (requires LibreOffice)
lit screenshot report.docx -o ./pages

# First 5 pages at high DPI
lit screenshot document.pdf --target-pages "1-5" --dpi 300 -o ./pages

# Specific pages only
lit screenshot document.pdf --target-pages "1,5,10" -o ./pages
```

---

## `lit is-complex`

Check whether a document needs OCR or heavier parsing — a cheap pre-parse pass over the text layer only (no rasterization, no OCR). See the [Document Complexity guide](/liteparse/guides/complexity/) for details.

```
lit is-complex [options] <file>
```

The command prints per-page JSON to **stdout**, a human-readable verdict to **stderr**, and exits **non-zero when any page needs OCR** — so it works as a shell predicate or a `jq` source.

### Arguments

| Argument | Description |
|----------|-------------|
| `file` | Path to the document file |

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `--compact` | Emit dense, whitespace-free JSON instead of pretty-printed | — |
| `--max-pages <n>` | Maximum pages to check | `1000` |
| `--target-pages <pages>` | Pages to check (e.g., `"1-5,10,15-20"`) | — (all pages) |
| `--password <password>` | Password for encrypted/protected documents | — |
| `-q, --quiet` | Suppress the stderr verdict | — |

### Examples

```bash
# Print the complexity verdict and per-page JSON
lit is-complex document.pdf

# Use as a shell predicate: only parse with --no-ocr when simple
lit is-complex document.pdf --quiet && lit parse document.pdf --no-ocr

# List the page numbers that need OCR
lit is-complex document.pdf --compact | jq '[.[] | select(.needs_ocr) | .page_number]'
```

---

## Global options

These options are available on all commands:

| Option | Description |
|--------|-------------|
| `-h, --help` | Show help for a command |
| `-V, --version` | Show version number |
