# Browser app

A single-page web app that runs LiteParse entirely in the browser. Pick a PDF, toggle OCR/screenshots, see the extracted text and pretty-printed JSON side by side. No server, no upload — the PDF stays on the user's machine.

[Claude Code transcript that built this](https://gisthost.github.io/?d64889bfc1b897fea3867adfec62ed89/index.html).

## Run it

From the repo root:

```bash
npm run dev:web      # vite dev server on http://localhost:5173
npm run build:web    # production build into dist-web/
npm run preview:web  # preview the built bundle
npm run test:web     # run the Playwright suite
```

The dev server is the intended entry — don't open `index.html` via `file://`, it can't fetch its own worker/WASM assets from that origin.

## What's in here

```
web/
  index.html          HTML shell + controls
  style.css           Layout (mobile-first; ≥900px breakpoint switches
                      text/JSON panes from stacked to side-by-side)
  main.ts             UI wiring: file → parse → render, clipboard,
                      drag-drop, screenshot rendering
  liteparse-browser.ts  Re-exports LiteParse from src/core/parser; serves
                        as the browser entry so Vite's aliases kick in
                        on the transitive imports.
  pdfjsImporter.ts    Browser-safe PDF.js loader. Sets workerSrc and
                      installs the ReadableStream.asyncIterator polyfill
                      Safari needs.
  pdfjs-renderer.ts   Replaces src/engines/pdf/pdfium-renderer.ts. Uses
                      PDF.js's own canvas renderer — no PDFium, no sharp.
                      Produces ImageData for OCR input and PNG bytes for
                      screenshots via OffscreenCanvas.convertToBlob.
  stubs/              Empty/minimal shims for Node modules that sneak
                      into the import graph (fs, path, axios, form-data,
                      file-type, http-simple OCR, convertToPdf, debug
                      logger, grid visualizer). Vite's resolve.alias +
                      a small resolveId plugin in vite.config.ts redirect
                      the real modules to these at build time.
  tests/              Playwright specs + fixtures
    fixtures/         sample-text.pdf, sample-scanned.pdf, corrupt.pdf,
                      not-a-pdf.txt. Regenerate with
                      `node web/tests/fixtures/generate.mjs`.
    app.spec.ts       End-to-end tests (13 cases, run on chromium + webkit)
```

## How the browser build works

The core LiteParse code in `src/` is almost entirely browser-compatible already — it's mostly pure TypeScript. The Node edges live in five places:

1. **PDFium native renderer** — replaced by PDF.js canvas rendering in `pdfjs-renderer.ts`.
2. **Format conversion** (LibreOffice/ImageMagick subprocesses) — stubbed; the browser build accepts PDF bytes only, non-PDF input throws a clear error.
3. **HTTP OCR engine** — stubbed; the browser uses Tesseract.js in a Web Worker.
4. **Debug logger / visualizer** — stubbed with no-ops; debug mode isn't exposed in the UI anyway.
5. **Process / fs references** — guarded in place with `typeof process !== "undefined"`.

`vite.config.ts` wires up the aliases that make all of this happen. A small `resolveId` plugin handles the file-level redirects, because Rollup's alias plugin matches the import specifier string — and `parser.ts` and `pdfjs.ts` reference the same files via different relative paths, so a proper resolver is needed.

## Gotchas I hit while building this

- **ArrayBuffer detachment.** PDF.js transfers the input Uint8Array's backing buffer to its worker. A second loader in the same parse (e.g. the OCR renderer asking for the same bytes) sees zero bytes. Fix: `src/engines/pdf/pdfjs.ts` now keeps a fresh copy in `currentPdfData` so the pdf.js transfer can't detach it.
- **Safari < 17 ReadableStream.** PDF.js uses `for await (const v of readableStream)` internally. Older WebKit ships ReadableStream but not its async iterator — throws "undefined is not a function (near '...value of readableStream...')". `pdfjsImporter.ts` polyfills `Symbol.asyncIterator` before the pdf.mjs import.
- **page.cleanup().catch() in the vendored pdf.mjs.** In recent bundles `page.cleanup()` returns `undefined` rather than a Promise, so chaining `.catch()` throws. The renderer duck-types the return value before awaiting.
- **tesseract.js CJS interop.** Tesseract.js is CommonJS. Vite needs it in `optimizeDeps.include` (not exclude) so the dep optimizer produces proper named ESM exports.
- **Image bounds.** PDFium exposes `_FPDFPageObj_GetBounds`; PDF.js doesn't provide a clean equivalent. `extractImageBounds` returns `[]` in the browser, so the parser falls back to full-page OCR whenever a page has little or no native text. Good enough for the common "scanned page" case.

## Tests

13 Playwright cases, each driven by the red/green TDD plan in `plan.md`. Each test locks in one behavior:

1. App shell loads (title, controls, textareas, copy buttons)
2. Parse button is disabled until a file is picked
3. Non-PDF files rejected with a clear error
4. Parse text PDF with OCR off, render text + JSON
5. Drop zone click-or-drop interaction, long filenames truncate
6. "Parsing…" status indicator appears and clears
7. Scanned PDF with OCR on extracts text via Tesseract (end-to-end WASM)
8. OCR toggle actually changes behavior (OCR-off → empty, OCR-on → populated)
9. Parser errors surface to `#status` and the button re-enables
10. Reparsing clears prior output before the new run
11. Screenshots render as blob-URL `<img>` tags; unchecking clears them
12. Mobile viewport (375px) stacks panes, touch targets ≥44px, no overflow
13. Copy buttons copy to clipboard and flash "Copied!" for 1.5s

Run on both Chromium and WebKit. Clipboard test is chromium-only (WebKit doesn't support Playwright's clipboard permission).

## Known trade-offs

- **Bundle size.** ~530 KB app + 2.3 MB pdf.worker.mjs. Unavoidable — PDF.js is that big. Tesseract loads lazily on first OCR run.
- **First OCR run is slow.** Tesseract.js fetches `eng.traineddata` from jsdelivr on first use (~10 MB). Runs from cache afterwards. A future improvement could stage traineddata under `public/` for offline-capable builds.
- **PDF-only.** DOCX/XLSX/etc. don't work in the browser — that would need LibreOffice-wasm (~50 MB) and isn't worth it. Non-PDF input throws a clear error.
- **No password-protected PDFs in v1.** Would require a password prompt; not wired.
