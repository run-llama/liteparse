---
title: Library Usage
description: Use LiteParse programmatically from TypeScript or Python.
sidebar:
  order: 1
---

LiteParse can be used as a library in your own code, not just from the CLI. There are packages for both TypeScript and Python.

## TypeScript

Install as a project dependency:

```bash
npm install @llamaindex/liteparse
# or
pnpm add @llamaindex/liteparse
```

### Parsing a document

```typescript
import { LiteParse } from "@llamaindex/liteparse";

const parser = new LiteParse({ ocrEnabled: true });
const result = await parser.parse("document.pdf");

// Full document text with layout preserved
console.log(result.text);

// Per-page data
for (const page of result.pages) {
  console.log(`Page ${page.pageNum}: ${page.textItems.length} text items`);
}
```

### JSON output with bounding boxes

```typescript
const parser = new LiteParse({ outputFormat: "json" });
const result = await parser.parse("document.pdf");

for (const page of result.json.pages) {
  for (const bbox of page.boundingBoxes) {
    console.log(`[${bbox.x1}, ${bbox.y1}] → [${bbox.x2}, ${bbox.y2}]`);
  }
}
```

### Configuration

Pass any config options to the constructor. You only need to specify what you want to override:

```typescript
const parser = new LiteParse({
  ocrEnabled: true,
  ocrServerUrl: "http://localhost:8828/ocr",
  ocrLanguage: "fra",
  dpi: 300,
  outputFormat: "json",
  targetPages: "1-10",
  password: "secret",        // for encrypted/protected documents
});
```

### Buffer / Uint8Array input

You can pass raw bytes directly instead of a file path. PDF buffers are parsed with **zero disk I/O** — no temp files are written:

```typescript
import { readFile } from "fs/promises";

const parser = new LiteParse();

// From a file read
const pdfBytes = await readFile("document.pdf");
const result = await parser.parse(pdfBytes);

// From an HTTP response
const response = await fetch("https://example.com/document.pdf");
const buffer = Buffer.from(await response.arrayBuffer());
const result2 = await parser.parse(buffer);
```

Non-PDF buffers (images, Office documents) are written to a temp directory for format conversion. You can control the temp directory with the `LITEPARSE_TMPDIR` environment variable.

### Screenshots

Generate page images as buffers — useful for sending to LLMs or saving to disk. Accepts file paths, `Buffer`, or `Uint8Array`:

```typescript
const parser = new LiteParse();
const screenshots = await parser.screenshot("document.pdf");

for (const shot of screenshots) {
  console.log(`Page ${shot.pageNum}: ${shot.width}x${shot.height}`);
  // shot.imageBuffer contains the raw PNG/JPG data
}

// Also works with buffer input
const pdfBytes = await readFile("document.pdf");
const shots = await parser.screenshot(pdfBytes, [1, 2, 3]);
```

### Environment variables

| Variable | Description |
|----------|-------------|
| `TESSDATA_PREFIX` | Path to a directory containing Tesseract `.traineddata` files. For offline/air-gapped environments. Also available as the `tessdataPath` config option. |
| `LITEPARSE_TMPDIR` | Override the temp directory for format conversion. Defaults to `os.tmpdir()`. |

See the [API reference](/liteparse/api/) for full type details.

---

## Python

The Python package wraps the LiteParse CLI, so the Node.js CLI must be installed first.

### Installation

```bash
# 1. Install the CLI (required)
npm install -g @llamaindex/liteparse

# 2. Install the Python package
pip install liteparse
```

<Aside type="caution">
  The Python package calls the LiteParse CLI under the hood. Make sure `lit` is available on your PATH before using the Python library.
</Aside>

### Parsing a document

```python
from liteparse import LiteParse

parser = LiteParse()
result = parser.parse("document.pdf")

# Full document text
print(result.text)

# Per-page data
for page in result.pages:
    print(f"Page {page.pageNum}: {len(page.textItems)} text items")
```

### Configuration

Options can be set on the constructor (applied to all parse calls) or per-call:

```python
# Constructor-level defaults
parser = LiteParse(
    ocr_enabled=True,
    ocr_server_url="http://localhost:8828/ocr",
    ocr_language="fra",
    dpi=300,
    password="secret",  # for encrypted/protected documents
)

# Per-call options
result = parser.parse("document.pdf", target_pages="1-5")
```

### Parsing from bytes

If you already have file contents in memory (e.g. from a web upload):

```python
result = parser.parse_bytes(pdf_bytes, filename="upload.pdf")
print(result.text)
```

### Batch parsing

For multiple files, batch mode reuses the PDF engine and is significantly faster:

```python
result = parser.batch_parse(
    input_dir="./documents",
    output_dir="./output",
    recursive=True,
    extension_filter=".pdf",
)

print(f"Parsed {result.success_count} files in {result.total_time_seconds}s")
```
