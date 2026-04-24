Short answer: **no, not as-is. Yes, with targeted changes** — and the core text-extraction path is closer to browser-ready than it looks.

**What blocks it today**

From `package.json`: `"engines": { "node": ">=18.0.0" }`, Node-style `bin` entries, ESM. Grepping the source for Node built-ins, the Node-coupled files are:

| File | Node dep | Purpose |
|---|---|---|
| `engines/pdf/pdfjs.ts` | `node:fs/promises` | Reading PDF from a path |
| `engines/pdf/pdfium-renderer.ts` | `fs`, `sharp` | Page rasterization for OCR |
| `engines/ocr/http-simple.ts` | `fs` (file upload) | HTTP OCR client |
| `conversion/convertToPdf.ts` | `fs`, `path`, `os`, `child_process` | LibreOffice/ImageMagick conversion |
| `processing/gridDebugLogger.ts` | `fs`, `path` | Debug tracing (optional) |
| `processing/gridVisualizer.ts` | `fs`, `sharp` | Debug image output (optional) |
| `engines/ocr/tesseract.ts` | `process.env.TESSDATA_PREFIX` | Offline tessdata lookup |
| `cli/parse.ts` | `commander`, `fs` | CLI — irrelevant for library use |

And from `dependencies`: `sharp` is a native Node addon (libvips bindings), `file-type` v21 is Node-only, `p-limit`'s package manifest declares `browser: false` (though in practice it's pure ESM and does work).

The only *hard* blocker in that list is `sharp`. Everything else is either a straight swap or isolated to a feature you can disable.

**What's already browser-shaped**

The big pieces:
- **PDF.js** — the vendored `pdf.mjs` is the normal browser build. `pdfjsImporter.ts` installs stubs for `DOMMatrix`, `Path2D`, `ImageData` *because it's running in Node*; a browser has those natively, so the stubs would just get skipped. The worker file `pdf.worker.mjs` is already a standard ES module worker.
- **Tesseract.js** — originally designed for browsers. Runs via WASM + Web Workers. Accepts `ImageData`, `HTMLCanvasElement`, `Blob`, `File`, or `ArrayBuffer` directly.
- **@hyzyla/pdfium** — WASM-based, declares `"browser": true` in its manifest.
- **Everything in `src/processing/`** — grid projection, bbox building, text cleanup, coordinate math, the Adobe Glyph List tables. ~1,650 lines in `gridProjection.ts` alone, zero Node imports. This is the interesting 80% of the library and it would port without a single line change.
- **`src/core/parser.ts`** — zero Node built-ins. Just orchestrates the other pieces.

**The concrete changes you'd need**

1. **Replace `sharp`**. In `pdfium-renderer.ts` it's used to wrap PDFium's raw RGBA output as PNG. Browser swap: either use `OffscreenCanvas.convertToBlob()` / `canvas.toDataURL()`, or — cleaner — **skip PNG encoding entirely** and hand the raw RGBA to Tesseract.js as `ImageData`, which it accepts natively. That actually makes the browser version faster than the Node one, which currently encodes PNG only to have Tesseract decode it again.

2. **Gate the path-based `loadDocument` branch** (`pdfjs.ts:668`). The `Uint8Array` branch already works unchanged; you'd just throw a clear error if someone passes a string in a browser context. `File`/`Blob` → `Uint8Array` is one `arrayBuffer()` call at the call site.

3. **Drop or stub `conversion/convertToPdf.ts`**. It shells out to LibreOffice and ImageMagick. No browser equivalent short of shipping a WASM LibreOffice (it exists, but it's ~50 MB). Realistic approach: browser build supports PDF and images only, throws on `.docx`/`.pptx`/`.xlsx`.

4. **Guard `process.env` and `process.stderr.write`** behind `typeof process !== 'undefined'`. Two sites, five lines total.

5. **Bundler asset handling**. `pdf.worker.mjs`, the WASM blobs (`jbig2.wasm`, `openjpeg.wasm`, `qcms_bg.wasm`), the CMap tables, standard fonts, and Tesseract's `eng.traineddata` all need to be resolvable as URLs at runtime. Vite / webpack / esbuild all handle this with `import.meta.url` + an asset plugin. The existing `pdfjsImporter.ts` already uses `new URL(..., import.meta.url)` — that pattern works in browsers.

6. **Debug-only files** (`gridDebugLogger`, `gridVisualizer`) — tree-shake them out of the browser bundle or stub the write calls.

**How much work**

Rough sketch:
- Browser build producing correct text extraction (no OCR) on a `Uint8Array`-in-memory PDF: a day, maybe two. The changes are mechanical.
- Plus OCR: another half-day once the rasterizer is rewired to skip PNG encoding.
- Feature parity including Office conversion: not worth it; restrict the browser build to PDF + images.

**Why nobody has done it yet is probably intentional**

The library has a CLI-first shape — `lit parse document.pdf` is the headline usage — and LlamaIndex clearly positions this as local tooling paired with their cloud product (LlamaParse) for the harder cases. A browser build would overlap more directly with things like PDF.js-powered web viewers and compete with it for the same niche. But there's no technical reason it couldn't exist; the core algorithm is pure, dependency-free TypeScript, and the Node coupling is at the edges.
