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

## Markdown Output

LiteParse can render documents directly to Markdown including headings, tables, lists,
images, and links reconstructed from the spatial layout. Great for feeding LLMs
and RAG pipelines. The rendered Markdown is returned on `result.text`:

```typescript
const parser = new LiteParse({
  outputFormat: 'markdown',     // "json" | "text" | "markdown"
  imageMode: 'placeholder',     // "placeholder" | "off" | "embed"
  extractLinks: true,           // render [text](url) link syntax (default: true)
});
const result = await parser.parse('document.pdf');
console.log(result.text); // rendered Markdown
```

> Reconstruction quality varies with document complexity.

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
  outputFormat: 'json',          // "json" | "text" | "markdown"
  imageMode: 'placeholder',      // Markdown image handling: "placeholder" | "off" | "embed"
  extractLinks: true,            // Render [text](url) links in markdown output
  preserveVerySmallText: false,  // Keep tiny text
  includeTextMetadata: false,    // Opt in to MCID, font metrics, colors, char codes, and tsg
  password: undefined,           // Password for protected documents
  quiet: false,                  // Suppress progress output
  numWorkers: 4,                 // Concurrent OCR workers
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

## Document Complexity

Before committing to a full parse, check whether a document needs OCR or heavier
processing. `isComplex` is a cheap, text-layer-only pass that returns one entry per page
with a `needsOcr` verdict and the signals behind it — useful for routing documents to
different pipelines, rejecting ones you can't handle, or estimating cost.

```typescript
const parser = new LiteParse();
const pages = await parser.isComplex('document.pdf');

if (pages.some((p) => p.needsOcr)) {
  // Route to the OCR-enabled pipeline
  const result = await parser.parse('document.pdf');
} else {
  // Cheap path — skip OCR entirely
  const result = await new LiteParse({ ocrEnabled: false }).parse('document.pdf');
}

// Inspect why specific pages were flagged
for (const page of pages.filter((p) => p.needsOcr)) {
  console.log(`Page ${page.pageNumber}: ${page.reasons.join(', ')}`);
}
```

`reasons` is one of `"scanned"`, `"no-text"`, `"sparse-text"`, `"embedded-images"`,
`"garbled"`, or `"vector-text"`. Raw bytes work here too.

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
lit is-complex document.pdf
```
