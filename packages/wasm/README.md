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
`"garbled"`, or `"vector-text"`.

## Config options

All optional, camelCase:

| Option | Type | Default | Description |
|---|---|---|---|
| `ocrLanguage` | `string` | `"eng"` | Language code passed to the OCR engine |
| `ocrEnabled` | `boolean` | `true` | Run OCR on text-sparse pages |
| `maxPages` | `number` | `1000` | Stop after this many pages |
| `targetPages` | `string` | — | e.g. `"1-5,10,15-20"` |
| `dpi` | `number` | `150` | Render DPI for OCR / screenshots |
| `outputFormat` | `"json" \| "text" \| "markdown"` | `"json"` | Output format; `"markdown"` returns rendered Markdown on `result.text` |
| `imageMode` | `"off" \| "placeholder" \| "embed"` | `"placeholder"` | How raster images are surfaced in markdown output |
| `extractLinks` | `boolean` | `true` | Render hyperlink annotations as `[text](url)` in markdown output |
| `extractVectorGraphics` | `boolean` | `false` | Include page-scoped shapes and merged horizontal/vertical lines |
| `preserveVerySmallText` | `boolean` | `false` | Keep tiny text that's normally filtered |
| `password` | `string` | — | Password for protected PDFs |
| `quiet` | `boolean` | `false` | Suppress progress logging |
| `ocrEngine` | `object` | — | JS-side OCR engine (see below) |

## OCR in the browser

The native HTTP-OCR and Tesseract backends are not available in the browser. To use OCR, pass an object with a `recognize` method:

```ts
const parser = new LiteParse({
  ocrEnabled: true,
  ocrLanguage: "eng",
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
