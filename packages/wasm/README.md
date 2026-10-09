# @llamaindex/liteparse-wasm

Browser/WebAssembly build of [LiteParse](https://github.com/run-llama/liteparse) — a fast, lightweight PDF parser with spatial text extraction.

This package runs entirely in the browser. No server, no cloud calls.

## Install

```sh
npm install @llamaindex/liteparse-wasm
```

## Quick start

```ts
import init, { LiteParse } from "@llamaindex/liteparse-wasm";

// Load the wasm module (point at the file shipped with the package).
await init();

const parser = new LiteParse({
  ocrEnabled: false, // OCR requires a JS-side engine (see below)
  outputFormat: "json",
});

// `data` is a Uint8Array (e.g. from fetch / File / drag-drop).
const bytes = new Uint8Array(await file.arrayBuffer());
const result = await parser.parse(bytes);

console.log(result.text);          // full document text
console.log(result.pages[0]);      // per-page items with bboxes
```

## Document complexity

Before committing to a full parse, check whether a document needs OCR or heavier
processing. `isComplex` is a cheap, text-layer-only pass that returns one entry per page
with a `needsOcr` verdict and the signals behind it — useful for routing documents or
deciding whether the JS-side OCR engine is worth wiring up.

```ts
const parser = new LiteParse({ ocrEnabled: false });
const bytes = new Uint8Array(await file.arrayBuffer());
const pages = await parser.isComplex(bytes);

if (pages.some((p) => p.needsOcr)) {
  // This document would benefit from OCR — see "OCR in the browser" below
  for (const page of pages.filter((p) => p.needsOcr)) {
    console.log(`Page ${page.pageNumber}: ${page.reasons.join(", ")}`);
  }
}
```

`reasons` is one of `"scanned"`, `"no-text"`, `"sparse-text"`, `"embedded-images"`,
`"garbled"`, `"vector-text"`, or `"annotation-text"`.

## Config options

All optional, camelCase:

| Option | Type | Default | Description |
|---|---|---|---|
| `ocrLanguage` | `string` | `"eng"` | Language code passed to the OCR engine |
| `ocrEnabled` | `boolean` | `true` | Run OCR on text-sparse pages |
| `maxPages` | `number` | `1000` | Stop after this many pages |
| `targetPages` | `string` | — | e.g. `"1-5,10,15-20"` |
| `extractScreenshots` | `boolean` | `false` | Return parsed pages as PNG bytes on `result.screenshots` |
| `dpi` | `number` | `150` | Render DPI for OCR / screenshots |
| `outputFormat` | `"json" \| "text" \| "markdown"` | `"json"` | Output format; `"markdown"` returns rendered Markdown on `result.text` |
| `imageMode` | `"off" \| "placeholder" \| "embed"` | `"placeholder"` | How raster images are surfaced in markdown output |
| `extractLinks` | `boolean` | `true` | Render hyperlink annotations as `[text](url)` in markdown output |
| `extractVectorGraphics` | `boolean` | `false` | Include page-scoped shapes and merged horizontal/vertical lines |
| `extractAnnotations` | `boolean` | `false` | Include page annotations and their metadata/geometry in structured output |
| `extractStructureTree` | `boolean` | `false` | Include the tagged-PDF logical structure tree |
| `preserveVerySmallText` | `boolean` | `false` | Keep tiny text that's normally filtered |
| `password` | `string` | — | Password for protected PDFs |
| `quiet` | `boolean` | `false` | Suppress progress logging |
| `numWorkers` | `number` | `1` | Maximum concurrent OCR calls |
| `ocrEngine` | `object` | — | JS-side OCR engine (see below) |
| `onOcrPage` | `(page: ParsedPage) => void` | — | Receive merged OCR text and boxes during `parse()` |

## OCR in the browser

The native HTTP-OCR and Tesseract backends are not available in the browser. To use OCR, pass an object with a `recognize` method:

```ts
const parser = new LiteParse({
  ocrEnabled: true,
  ocrLanguage: "eng",
  // Use more than 1 only if the engine supports concurrent jobs.
  numWorkers: 2,
  ocrEngine: {
    /**
     * @param imageData PNG-encoded image bytes
     * @param width  rendered page width  in pixels
     * @param height rendered page height in pixels
     * @param language e.g. "eng"
     * @returns array of { text, bbox: [x1,y1,x2,y2], confidence }
     */
    async recognize(imageData, width, height, language) {
      // e.g. call a worker that wraps tesseract.js, or a remote OCR service
      return [
        { text: "Hello", bbox: [10, 20, 80, 40], confidence: 0.98 },
      ];
    },
  },
});
```

LiteParse keeps at most `numWorkers` calls to `recognize` active. When one call
finishes, LiteParse prepares and starts the next page without waiting for the
other calls. An OCR engine can send the calls to a Web Worker pool or to
concurrent HTTP requests. Results stay assigned to their source pages when
jobs finish out of order. Active calls can keep up to `numWorkers` rendered
page rasters in memory, so use a small value for high-DPI documents.

### Receive OCR pages before parsing ends

LiteParse merges each completed OCR result into its page on the caller thread.
Set `onOcrPage` to receive that page's text and boxes before `parse()` returns:

```typescript
import type { ParsedPage } from "@llamaindex/liteparse-wasm";

const previews = new Map<number, ParsedPage>();
const parser = new LiteParse({
  ocrEnabled: true,
  ocrEngine,
  numWorkers: 4,
  onOcrPage(page) {
    previews.set(page.pageNum, page);
    // Schedule a UI update here. Keep this callback short and synchronous.
  },
});
const result = await parser.parse(pdfBytes);
// Replace the previews with the final pages, which include document layout.
previews.clear();
for (const page of result.pages) previews.set(page.pageNum, page);
```

The callback runs once for each successful OCR page, including an empty OCR
result. It does not run for failed OCR jobs or pages that do not need OCR.
Calls can arrive out of order. Use the source `pageNum`, not the callback order.
Early delivery also works with `numWorkers: 1`.

The page contains merged, filtered text and boxes. It has no `blocks` or
`complexity`, and its `markdown` is empty. Document-wide layout still runs at
the end. Treat these pages as previews and use the final result for complete
output. Changes to a JS preview do not change the final Rust result.

Callbacks run synchronously on the caller thread. Their return values are
ignored; LiteParse does not await them. A thrown exception rejects `parse()`.
A later parse error can also invalidate earlier previews. The caller must
handle rejection and manage its own OCR worker pool. The callback applies to
`parse()` only, not to `ParseSession.nextBatch()`.

Without a callback, LiteParse still merges each OCR page as it finishes, but
does not create or serialize preview pages. The OCR failure policy is unchanged.

### Read OCR errors

`result.ocrErrors` lists failed OCR jobs as `{ pageNum, message }`, in source
page order. Page numbers start at 1 and stay the same when `targetPages` is set.
The list is empty when OCR is disabled or no OCR job failed. A successful OCR
job with no words is not an error. `pageErrors` remains separate: it reports
PDF extraction failures, not OCR failures.

```typescript
const parser = new LiteParse({
  ocrEnabled: true,
  ocrEngine,
  ocrFailureFatal: false,
});
const result = await parser.parse(pdfBytes);
for (const error of result.ocrErrors) {
  console.warn(`OCR failed on page ${error.pageNum}: ${error.message}`);
}
```

This field does not change the failure policy or retry failed pages. If
`parse()` rejects, it does not return a result. Use `ocrFailureFatal: false`
to receive partial results when all OCR jobs fail. Other fatal parse errors
can still reject the call. Node.js also uses `ocrErrors`; Rust and Python use
`ocr_errors`.

## Building from source

Requires Rust + [`wasm-pack`](https://rustwasm.github.io/wasm-pack/):

```sh
# from packages/wasm
npm run build           # web target (default)
npm run build:bundler   # for webpack/rollup/vite
npm run build:nodejs    # for node.js
```

Output goes to `pkg/`.

## License

Apache-2.0
