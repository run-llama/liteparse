# LiteParse Node.js

Node.js/TypeScript bindings for [LiteParse](https://github.com/run-llama/liteparse) — fast, lightweight PDF and document parsing with spatial text extraction.

## Installation

```bash
npm i @llamaindex/liteparse
```

This also installs the `lit` CLI command (use `npm i -g` for global access).

## Quick Start

```typescript
import { LiteParse } from '@llamaindex/liteparse';

const parser = new LiteParse();
const result = await parser.parse('document.pdf');
console.log(result.text);

// Access structured data
for (const page of result.pages) {
  console.log(`Page ${page.pageNum}: ${page.textItems.length} text items`);
}
```

## Configuration

All options are passed to the constructor:

```typescript
const parser = new LiteParse({
  ocrEnabled: true,              // Enable OCR (default: true)
  ocrLanguage: 'eng',           // Tesseract language code
  ocrServerUrl: undefined,       // HTTP OCR server URL (optional)
  tessdataPath: undefined,       // Path to tessdata directory (optional)
  maxPages: 1000,                // Max pages to parse
  targetPages: '1-5,10',        // Specific pages (optional)
  dpi: 150,                      // Rendering DPI
  preserveVerySmallText: false,  // Keep tiny text
  password: undefined,           // Password for protected documents
  quiet: false,                  // Suppress progress output
  numWorkers: 4,                 // Concurrent OCR workers
  inlineImages: false,           // Inline embedded images as base64 markdown images
});
```

## Parsing from Bytes

Pass a `Buffer` or `Uint8Array` directly — useful for HTTP responses or in-memory data:

```typescript
import { readFile } from 'fs/promises';

const pdfBytes = await readFile('document.pdf');
const result = await parser.parse(pdfBytes);
console.log(result.text);
```

## Screenshots

Generate PNG screenshots of document pages:

```typescript
const screenshots = parser.screenshot('document.pdf', [1, 2, 3]);
for (const s of screenshots) {
  console.log(`Page ${s.pageNum}: ${s.width}x${s.height}`);
  // s.imageBuffer contains PNG bytes
}
```

## Supported Formats

- PDF (`.pdf`)
- Microsoft Office (`.docx`, `.xlsx`, `.pptx`, etc.) — requires LibreOffice
- OpenDocument (`.odt`, `.ods`, `.odp`) — requires LibreOffice
- Images (`.png`, `.jpg`, `.tiff`, etc.) — requires ImageMagick
- And more!

## CLI

The npm package includes the `lit` CLI:

```bash
lit parse document.pdf
lit parse document.pdf --format json -o output.json
lit screenshot document.pdf -o ./screenshots
lit batch-parse ./input ./output
```
