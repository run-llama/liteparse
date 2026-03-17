---
title: Getting Started
description: Install LiteParse and parse your first document in under a minute.
sidebar:
  order: 1
---

LiteParse is an open-source PDF parsing tool that parses text with spatial layout information and bounding boxes. It runs entirely on your machine, which means no cloud dependencies, no LLMs, no API keys.

## Installation

```bash
# npm (recommended)
npm i -g @llamaindex/liteparse

# brew (macOS/Linux)
brew tap run-llama/liteparse
brew install llamaindex-liteparse
```

## Quick start

Once installed, you can start parsing immediately from the command line:

```bash
lit parse document.pdf

# Save output to a text file
lit parse document.pdf -o output.txt

# Get structured JSON with bounding boxes
lit parse document.pdf --format json -o output.json

# Parse only specific pages
lit parse document.pdf --target-pages "1-5,10,15-20"

# Parse an entire directory of PDFs into an output directory
lit batch-parse ./pdfs ./outputs

# Screenshot the pages of a PDF for LLM agents
lit screenshot document.pdf -o ./screenshots
```

## Next steps

- [OCR Servers](./ocr) — configure Tesseract, EasyOCR, PaddleOCR, or bring your own
- [Multi-format support](./multi-format) — parse DOCX, XLSX, PPTX, images, and more
- [Agent skill](./agent-skill) — use LiteParse as a coding agent skill
- [CLI reference](./cli-reference) — complete command and option reference
- [API reference](./api) — TypeScript library types and methods
