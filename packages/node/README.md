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
console.log(`Source document pages: ${result.totalPages}`);

// Access structured data
for (const page of result.pages) {
  console.log(`Page ${page.pageNum}: ${page.textItems.length} text items`);
}
```

### Bounded-memory parsing

For documents with many text items, consume page batches without retaining
earlier results:

```typescript
const parser = new LiteParse();
for await (const batch of parser.parseBatches('large.pdf', { batchSize: 20 })) {
  await processPages(batch.result.pages);
}
```

Each batch is an ordinary parse result covering `batch.startPage` through
`batch.endPage`, and becomes collectible as soon as you advance the iterator.
A non-PDF source is converted once, not once per batch.

Cross-page passes only see the pages in their own batch, so repeated
header/footer removal and image deduplication are batch-local and the output
can differ from `parse()`. Prefer `parse()` unless the size of the
materialized result is the problem.

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
  extractScreenshots: false,    // Return rendered pages
  screenshot: {
    format: 'png',               // 'png' (default) or packed 'rgb8'
    png: { compression: 'fast' }, // 'fast' (default), 'default', or 'best'
  },
  continueOnPageError: false,   // Skip broken pages and return pageErrors
  dpi: 150,                      // Rendering DPI
  outputFormat: 'json',          // "json" | "text" | "markdown"
  imageMode: 'placeholder',      // Markdown image handling: "placeholder" | "off" | "embed"
  extractImages: true,           // Extract image bytes + metadata (default: false)
  imageOutputDir: './images',    // Write images and return name/path metadata (optional)
  extractLinks: true,            // Render [text](url) links in markdown output
  keepHeadersFooters: false,     // Keep running headers/footers in markdown output
  extractVectorGraphics: false,  // Opt-in shapes + merged H/V lines per page
  extractAnnotations: false,     // Include page annotations in structured output
  extractFormFields: false,      // Include AcroForm widget fields and values
  extractStructureTree: false,   // Include tagged-PDF logical structure
  extractXfaPackets: false,      // Include raw XFA packets (name + XML content)
  extractContentBounds: false,   // Include per-page contentBounds union bbox
  detectScreenshotRects: false,  // Detect solid rects/lines in screenshots
  preserveVerySmallText: false,  // Keep tiny text
  extractTextMetadata: false,    // Opt in to MCID, font metrics, colors, char codes, and trailingSpaceGenerated
  password: undefined,           // Password for protected documents
  quiet: false,                  // Suppress progress output
  numWorkers: 4,                 // Concurrent OCR workers
});
```

When `extractImages` is true, image extraction is enabled. `imageOutputDir` requires
that explicit opt-in and writes the extracted bytes to disk. Each
`result.images` entry includes its page bbox, intrinsic pixel dimensions, rotation,
format, `name`, and `path`. Valid source JPEGs are preserved, exact duplicates reuse
one file, and JSON CLI output contains metadata only (no base64 image data).
`imageMode` controls Markdown presentation only and does not imply extraction. With
`extractImages: false`, lightweight Markdown placement refs are still collected and
`result.images` stays empty.

The Node CLI uses the same snake_case JSON schema as the Rust CLI; camelCase is
reserved for the programmatic Node API.

When `extractAnnotations` is enabled, each parsed page has an `annotations`
array containing the subtype, contents, author/title, PDF date strings,
viewport-space rectangle and quadpoint rectangles, and URI for external link
annotations. It is independent of `extractLinks`, which controls Markdown link
rendering. The field is omitted when extraction is disabled.

When `extractStructureTree` is enabled, each page has a `structureTree` containing
all tagged-PDF roots and recursive elements with type, ID, actual/alternate text,
title, typed attributes, MCIDs, children, and referenced link annotations. Untagged
pages have an empty `roots` array; the field is omitted when disabled.

Every result also carries the document's `/Info` `creator`/`producer` when
present. With `extractDocumentMetadata`, `result.docMeta` adds a provenance
object with dates, PDF version/security, signature state, incremental-save
markers, trailer ID comparison, the catalog's XMP packet (capped at 64 KiB;
skipped for sources over 16 MiB), and source size — off by default since it
streams the whole file, and absent for inputs converted from a non-PDF
format. These are API-level only, not in CLI JSON. With `extractContentBounds`
each page carries a `contentBounds` union bbox of its top-level content
objects. With `extractXfaPackets`, `result.xfaPackets` lists each raw XFA
packet (index, name, content length, XML content); non-XFA documents yield an
empty array. Screenshots draw form-field appearances into the raster and
report `isSolidFill` per page, plus detected solid `rects` when
`detectScreenshotRects` is enabled.

## Parsing from Bytes

Pass a `Buffer` or `Uint8Array` directly — useful for HTTP responses or in-memory data:

```typescript
import { readFile } from 'fs/promises';

const pdfBytes = await readFile('document.pdf');
const result = await parser.parse(pdfBytes);
console.log(result.text);
```

## Screenshots

Generate screenshots of document pages:

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
`"garbled"`, `"vector-text"`, or `"annotation-text"`. Raw bytes work here too.

## Supported Formats

- PDF (`.pdf`)
- Microsoft Office (`.docx`, `.xlsx`, `.pptx`, etc.) — requires LibreOffice
- OpenDocument (`.odt`, `.ods`, `.odp`) — requires LibreOffice
- Images (`.png`, `.jpg`, `.tiff`, etc.)
- And more!

## CLI

The npm package includes the `lit` CLI:

```bash
lit parse document.pdf
lit parse document.pdf --format json -o output.json
lit parse document.pdf --format json --extract-annotations
lit parse document.pdf --format json --extract-form-fields
lit screenshot document.pdf -o ./screenshots
lit batch-parse ./input ./output
lit is-complex document.pdf
```
