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

### Reading-order text

The `ordered` format groups nearby text into content blobs and sorts them in reading order, producing clean flowing text without spatial whitespace:

```typescript
const parser = new LiteParse({ outputFormat: "ordered" });
const result = await parser.parse("document.pdf");

// Clean text without column padding or whitespace formatting
console.log(result.text);
```

This will save tokens in LLM applications, at the risk of losing some layout cues in the overall text structure.

### JSON output with bounding boxes

```typescript
const parser = new LiteParse({ outputFormat: "json" });
const result = await parser.parse("document.pdf");

for (const page of result.json?.pages || []) {
  for (const item of page.textItems) {
    console.log(`[${item.x}, ${item.y}] → [${item.x + item.width}, ${item.y + item.height}] ${item.text}`);
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

The Python package is a wrapper around the LiteParse Node.js CLI. **Node.js (>= 18) must be installed** on your system.

### Installation

First, install the LiteParse CLI:

```bash
npm install -g @llamaindex/liteparse
```

Then install the Python package:

```bash
pip install liteparse
```

<Aside type="caution">
  The Python package calls the LiteParse CLI under the hood via subprocess. Node.js (>= 18) is required. While the package can auto-install the CLI via `npm install -g @llamaindex/liteparse` on first use, it is recommended to install it separately beforehand.
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

The `LiteParse` constructor accepts `cli_path` (to override CLI auto-detection) and `install_if_not_available` (to control auto-install behavior). All parsing options are passed per-call:

```python
parser = LiteParse()

result = parser.parse(
    "document.pdf",
    ocr_enabled=True,
    ocr_server_url="http://localhost:8828/ocr",
    ocr_language="fra",
    dpi=300,
    target_pages="1-5",
    password="secret",  # for encrypted/protected documents
)
```

### Parsing from bytes

If you already have file contents in memory (e.g. from a web upload), pass them directly to `parse()`:

```python
with open("document.pdf", "rb") as f:
    pdf_bytes = f.read()

result = parser.parse(pdf_bytes)
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

print(f"Output written to: {result.output_dir}")
```
