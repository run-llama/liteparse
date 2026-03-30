---
title: Getting Started
description: Install LiteParse and parse your first document in under a minute.
sidebar:
  order: 1
---

## Installation

Install LiteParse globally via npm to use the `lit` command anywhere:

```bash
npm i -g @llamaindex/liteparse
```

For macOS and Linux users, LiteParse can be also installed via `brew`:

```bash
brew tap run-llama/liteparse
brew install llamaindex-liteparse
```

## Quick start

Once installed, you can start parsing from the command line:

```bash
# Parse a PDF and print text to stdout
lit parse document.pdf

# Save output to a file
lit parse document.pdf -o output.txt

# Get structured JSON with bounding boxes
lit parse document.pdf --format json -o output.json

# Reading-order text
lit parse document.pdf --format ordered

# Parse only specific pages
lit parse document.pdf --target-pages "1-5,10,15-20"
```

### Batch parsing

Parse an entire directory of documents at once:

```bash
lit batch-parse ./pdfs ./outputs
```

### Screenshots

Generate page images from a PDF for LLM agents or visual workflows:

```bash
lit screenshot document.pdf -o ./screenshots
```

## Next steps

- [Library usage](/liteparse/guides/library-usage/): Use LiteParse programmatically from TypeScript or Python.
- [OCR configuration](/liteparse/guides/ocr/): Configure Tesseract, use an external OCR server, or bring your own.
- [Multi-format support](/liteparse/guides/multi-format/): Parse DOCX, XLSX, PPTX, images, and more.
- [Agent skill](/liteparse/guides/agent-skill/): Add LiteParse as a skill for coding agents.
- [CLI reference](/liteparse/cli-reference/): Complete command and option reference.
