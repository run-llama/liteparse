# Plan: LiteParse as a Browser Web App

## Goal
A standalone web app — user opens `index.html`, picks a PDF from their machine, toggles OCR on/off, and sees LiteParse's parsed output (text + JSON). Everything runs client-side; no server, no upload.

## Approach
Use **Vite** as the bundler. It handles the three hard things we need:
- Resolves `new URL("...", import.meta.url)` asset references for the PDF.js worker, WASM blobs, CMap tables, and standard fonts.
- Tree-shakes out the Node-only branches (conversion, debug logger, gridVisualizer, http-simple OCR).
- Serves a dev loop with HMR.

The `notes.md` analysis is correct: ~80% of the codebase (`src/processing/**`, `src/core/parser.ts`, most of `src/engines/pdf/pdfjs.ts`) is already browser-compatible. The work is at the edges — replace `sharp`, gate `fs` branches, and swap the few `process.*` references.

We will **not** fork the library. All browser-specific code goes in a new `web/` directory plus a handful of minimal shims. Shared source files are edited in place only when the change is safe for Node (e.g. `typeof process !== "undefined"` guards).

## Scope (v1)
- ✅ PDF input only. DOCX/XLSX/images are out — the browser has no LibreOffice.
- ✅ OCR on/off toggle. Built-in Tesseract.js in the browser (uses its own Web Workers + WASM, fetches traineddata from the CDN on first run).
- ✅ Text and JSON output views.
- ✅ Screenshots (`renderPageToBuffer`) — swap `sharp`'s PNG encode for `OffscreenCanvas.convertToBlob()`. Same `Uint8Array` PNG output shape as the Node path, so the public API stays intact.
- ❌ HTTP OCR server backend — irrelevant for local-only use.
- ❌ Password-protected PDFs — could add a password input later; not in v1.

## Architecture

```
web/
  index.html          # Page shell, file input, toggles, result area
  main.ts             # UI wiring, calls LiteParseBrowser
  liteparse-browser.ts # Browser-safe parser entry (thin wrapper over core parser)
  pdfjs-renderer.ts   # Browser replacement for pdfium-renderer.ts (no sharp, no PDFium — pure PDF.js canvas render)
vite.config.ts        # Asset plugin, entry = web/index.html
```

Everything else — `src/core/parser.ts`, `src/processing/**`, `src/output/**`, `src/engines/pdf/pdfjs.ts`, `src/engines/ocr/tesseract.ts`, `src/vendor/pdfjs/**` — is imported **as-is** by `liteparse-browser.ts`.

We drop `@hyzyla/pdfium` from the browser bundle entirely. PDF.js already has a full canvas-based renderer built in, and we're already loading it for text extraction. Using it for rendering too means one PDF engine instead of two, no extra WASM blob, and no PDFium-in-Vite config risk.

### Data flow in the browser
1. User picks a file → `File` object.
2. `file.arrayBuffer()` → `Uint8Array`.
3. `new LiteParseBrowser({ ocrEnabled }).parse(bytes)` — mirrors the Node API.
4. Under the hood: the existing `LiteParse` class's Uint8Array branch is reused. We skip the `convertToPdf` call entirely by detecting PDF bytes up front (magic number check `%PDF-`). Non-PDF bytes are rejected with a clear message.
5. Results rendered into the DOM.

## Detailed changes

### 1. Replace `sharp` + PDFium with PDF.js's own canvas renderer
**Problem:** The Node renderer (`pdfium-renderer.ts`) does two things that don't port:
1. Calls into `@hyzyla/pdfium` (WASM) to rasterize a page to raw RGBA.
2. Uses `sharp` to encode that RGBA as PNG.

Both consumers of its output — Tesseract OCR and `LiteParse.screenshot()` — don't actually care *which* engine rendered the page. They just want pixels.

**Fix:** Skip PDFium entirely in the browser and use PDF.js's built-in `page.render({ canvasContext, viewport })`. PDF.js is already loaded for text extraction, so this is free. Create `web/pdfjs-renderer.ts` that implements the same surface as `PdfiumRenderer` (`loadDocument`, `renderPageToBuffer`, `extractImageBounds`, `close`), backed entirely by PDF.js:

```ts
// web/pdfjs-renderer.ts
export class PdfJsRenderer {
  private doc?: PdfJsDocument;

  async loadDocument(input: Uint8Array) {
    const { fn: getDocument } = await importPdfJs();
    this.doc = await getDocument({ data: input }).promise;
  }

  async renderPageToCanvas(pageNum: number, dpi: number) {
    const page = await this.doc!.getPage(pageNum);
    const scale = dpi / 72;
    const viewport = page.getViewport({ scale });
    const canvas = new OffscreenCanvas(viewport.width, viewport.height);
    const ctx = canvas.getContext("2d")!;
    await page.render({ canvasContext: ctx, viewport }).promise;
    return { canvas, ctx, width: viewport.width, height: viewport.height };
  }

  async renderPageToBuffer(_input, pageNum, dpi = 150): Promise<Uint8Array> {
    const { canvas } = await this.renderPageToCanvas(pageNum, dpi);
    const blob = await canvas.convertToBlob({ type: "image/png" });
    return new Uint8Array(await blob.arrayBuffer());
  }

  async renderPageToImageData(pageNum: number, dpi: number): Promise<ImageData> {
    const { ctx, width, height } = await this.renderPageToCanvas(pageNum, dpi);
    return ctx.getImageData(0, 0, width, height);
  }
  // ...
}
```

- Returns `Uint8Array` for `renderPageToBuffer` (we alias `Buffer` → `Uint8Array` via the Buffer shim, so existing `Promise<Buffer>` callers still type-check).
- OCR zero-copy path: `renderPageToImageData` returns `ImageData` straight from the canvas, which Tesseract.js accepts natively. Small edit to `parser.ts`'s OCR branch to call it instead of `renderPageToBuffer` when the engine exposes it.
- `extractImageBounds` is the one piece PDF.js doesn't give us cleanly — PDFium's `_FPDFPageObj_GetBounds` is gone. Two options:
  - **(a)** Use PDF.js's `page.getOperatorList()` and walk the ops looking for `paintImageXObject` / `paintJpegXObject`, pulling their transform matrices from the graphics state. Doable but ~50 lines of fiddly matrix math.
  - **(b)** Return `[]` (no embedded-image bounds). The parser's OCR path then falls back to full-page OCR whenever `textLength < 100` (the existing "low-text page" heuristic in `parser.ts:370`). We lose the "OCR only the region around a specific image" optimization; we keep the "OCR the whole page if it's a scan" behavior, which is the common case.
  - **Go with (b) for v1**, ship, revisit if benchmarks show full-page OCR is too slow on image-heavy documents.

**Wiring:** Vite `resolve.alias` redirects `src/engines/pdf/pdfium-renderer.ts` → `web/pdfjs-renderer.ts`. Existing Node import paths in `pdfjs.ts` stay untouched.

**Side benefit:** drops `@hyzyla/pdfium` from the browser dependency graph entirely. One fewer WASM blob to serve, one fewer Vite asset-loading risk to debug, no `sharp` alias needed. Screenshots work out of the box — PDF.js's canvas render is the screenshot. The UI wraps the returned `Uint8Array` in a `Blob` → `URL.createObjectURL()` → `<img>` tag.

### 2. Gate the file-path branch in `pdfjs.ts`
`loadDocument(input: string | Uint8Array)` calls `fs.readFile` when given a string. In the browser, wrap that in:
```ts
if (typeof input === "string") {
  if (typeof window !== "undefined") {
    throw new Error("File paths not supported in browser — pass a Uint8Array");
  }
  // existing Node branch
}
```
Vite still pulls `node:fs/promises` into the bundle at parse time. Fix with **`resolve.alias`** redirecting `node:fs/promises` to an empty stub module (also aliased for `fs`, `path`, `os`, `child_process`, `form-data`). The string branch is dead code in the browser — the stubs just need to prevent bundle-time resolution failures.

### 3. Swap `convertToPdf` out of the import graph
`src/core/parser.ts` imports `../conversion/convertToPdf.js`. That module transitively pulls in `fs`, `path`, `os`, `child_process`, `file-type`.

**Fix:** Vite `resolve.alias` for `../conversion/convertToPdf.js` → `web/stubs/convert-stub.ts`. The stub exports:
- `convertToPdf` / `convertBufferToPdf` — throw "only PDF supported".
- `guessExtensionFromBuffer` — inspect the first 4 bytes (`%PDF`) and return `".pdf"` or throw.
- `cleanupConversionFiles` — no-op.

Parser's code path becomes: buffer input → `guessExtensionFromBuffer` → `.pdf` → `loadDocument(Uint8Array)`. The non-PDF branch never fires.

### 4. Guard `process` accesses
Two sites:
- `tesseract.ts:15` — `process.env.TESSDATA_PREFIX`. Wrap: `typeof process !== "undefined" ? process.env.TESSDATA_PREFIX : undefined`. Edit in place.
- `gridProjection.ts:2243` / `gridDebugLogger.ts:182` — debug-only writes. These only fire when `debug.enabled` is set. Safe as-is since UI never enables debug, but still wrap with `typeof process !== "undefined"` for belt-and-braces.

### 5. Exclude debug modules
`gridDebugLogger.ts` and `gridVisualizer.ts` import `fs`, `sharp`, `path`. They're imported eagerly from `gridProjection.ts`. Because they're only *used* when `debug.enabled` is true, tree-shaking may or may not eliminate them.

**Fix:** Vite aliases for both files → empty stubs that export the same names with no-op implementations. Zero code change to `gridProjection.ts`.

### 6. HTTP OCR engine
`http-simple.ts` imports `fs`, `form-data`, `axios`. `core/parser.ts` imports it at the top. We never instantiate it in the browser (config won't set `ocrServerUrl`), but the import still resolves.

**Fix:** Vite alias → stub exporting a class that throws on construction. Browser users can't reach it.

### 7. PDF.js importer
`pdfjsImporter.ts` uses `fileURLToPath` and `dirname` from Node. The DOMMatrix/Path2D/ImageData stubs it installs are skipped in browsers (they already exist). The `new URL("../../vendor/pdfjs/pdf.mjs", import.meta.url)` pattern works fine in a Vite build — Vite rewrites it to a bundled asset URL.

**Fix:** Write a `web/pdfjsImporter.ts` that drops the Node stubs and the `fileURLToPath` call, returning just `{ fn, dir }` where `dir` is a URL string (used by `pdfjs.ts` as the base for CMap/font/WASM URLs). Vite alias → redirect `src/engines/pdf/pdfjsImporter.ts` to this file.

Note: `pdfjs.ts` passes `CMAP_URL = ${PDFJS_DIR}/cmaps/` to PDF.js as a string. In the browser, `PDFJS_DIR` needs to be an HTTP URL the browser can fetch. The cleanest path: put `src/vendor/pdfjs/cmaps/`, `standard_fonts/`, `*.wasm` under `public/pdfjs/` (or use Vite's `publicDir`), and set `PDFJS_DIR = "/pdfjs"` at build time. Worker is loaded via `new URL("../../vendor/pdfjs/pdf.worker.mjs", import.meta.url)` in `pdfjs.ts` — already browser-compatible.

Actually, `pdfjs.ts` doesn't explicitly reference the worker — PDF.js handles its own worker URL via `workerSrc`. We'll set `pdfjs.GlobalWorkerOptions.workerSrc = new URL("../../vendor/pdfjs/pdf.worker.mjs", import.meta.url).href` in the browser importer, once, before calling `getDocument`.

### 8. Tesseract traineddata
Tesseract.js defaults to fetching `eng.traineddata` from `cdn.jsdelivr.net` on first recognize. That works out of the box in a browser with network access. We won't set `langPath` — accept the CDN default for v1. Document that first-use OCR requires one-time network access for language data.

### 9. UI (`web/index.html` + `web/main.ts` + `web/style.css`)

Single page, no framework. Both text and pretty-printed JSON are always rendered — no format toggle. Each sits in its own `<textarea readonly>` with a copy-to-clipboard button above it. Screenshots render as a strip below. Mobile-friendly layout: single-column stack with touch-sized controls at narrow widths, side-by-side textareas on wider viewports.

```html
<main class="app">
  <header>
    <h1>LiteParse</h1>
    <p>Parse PDFs in your browser.</p>
  </header>

  <section class="controls">
    <label class="file">
      <input type="file" accept="application/pdf" id="file">
      <span>Choose a PDF…</span>
    </label>
    <label class="toggle"><input type="checkbox" id="ocr" checked> Run OCR</label>
    <label class="toggle"><input type="checkbox" id="shots"> Render page screenshots</label>
    <button id="parse" disabled>Parse</button>
  </section>

  <div id="status" role="status" aria-live="polite"></div>

  <section class="results">
    <div class="pane">
      <div class="pane-header">
        <h2>Text</h2>
        <button class="copy" data-target="text-output">Copy</button>
      </div>
      <textarea id="text-output" readonly placeholder="Extracted text will appear here…"></textarea>
    </div>
    <div class="pane">
      <div class="pane-header">
        <h2>JSON</h2>
        <button class="copy" data-target="json-output">Copy</button>
      </div>
      <textarea id="json-output" readonly placeholder="Pretty-printed JSON will appear here…"></textarea>
    </div>
  </section>

  <section id="screenshots" class="screenshots" hidden></section>
</main>
```

**Layout (CSS):**
- Base: single column stack, controls wrap, textareas full-width, ~40vh tall each.
- `@media (min-width: 900px)`: `.results` becomes a two-column grid so text and JSON sit side by side; textareas grow to `min(70vh, 600px)`.
- Use `box-sizing: border-box`, system font stack, `font-size: 16px` on inputs (stops iOS Safari auto-zoom).
- Buttons: min-height 44px, comfortable padding — touch-friendly.
- `textarea { width: 100%; font-family: ui-monospace, SFMono-Regular, monospace; resize: vertical; white-space: pre; overflow: auto; }` — `white-space: pre` preserves spatial layout without wrapping.
- Meta: `<meta name="viewport" content="width=device-width, initial-scale=1">` — non-negotiable.

**Interaction (`main.ts`):**
- Parse button disabled until a file is chosen.
- On parse: set status to "Parsing…", disable button, clear both textareas and screenshot strip (plus `URL.revokeObjectURL` for prior screenshots).
- Run parse with `outputFormat: "json"` always — we render both views from the single result (`result.text` → text textarea, `JSON.stringify(result.json, null, 2)` → JSON textarea).
- If screenshots checkbox is ticked, call `.screenshot(bytes)` after parse; wrap each `imageBuffer` in `new Blob([buf], { type: "image/png" })` → `URL.createObjectURL` → append `<img>` to `#screenshots`; unhide the section.
- Copy buttons: `navigator.clipboard.writeText(textarea.value)`; briefly flip button label to "Copied" for feedback (fall back gracefully if `clipboard` API unavailable — select the textarea and let the user copy manually).
- Errors: caught in the parse handler, rendered into `#status` with an `error` class, button re-enabled.
- Progress plumbing: wrap the parse call in a `console.error` capture that forwards log lines to `#status` (parser's log target is already `console.error`).

### 10. Vite config sketch
```ts
// vite.config.ts
export default {
  root: "web",
  publicDir: "../src/vendor/pdfjs",   // served at /pdfjs/*
  resolve: {
    alias: {
      "node:fs/promises": "/web/stubs/fs.ts",
      "node:url": "/web/stubs/url.ts",
      "fs": "/web/stubs/fs.ts",
      "path": "/web/stubs/path.ts",
      "os": "/web/stubs/os.ts",
      "child_process": "/web/stubs/empty.ts",
      "form-data": "/web/stubs/empty.ts",
      "axios": "/web/stubs/empty.ts",
      "file-type": "/web/stubs/file-type.ts",
      // File-level redirects:
      [path.resolve("src/engines/pdf/pdfium-renderer.ts")]: path.resolve("web/pdfjs-renderer.ts"),
      [path.resolve("src/engines/pdf/pdfjsImporter.ts")]: path.resolve("web/pdfjsImporter.ts"),
      [path.resolve("src/engines/ocr/http-simple.ts")]: path.resolve("web/stubs/http-simple.ts"),
      [path.resolve("src/conversion/convertToPdf.ts")]: path.resolve("web/stubs/convertToPdf.ts"),
      [path.resolve("src/processing/gridDebugLogger.ts")]: path.resolve("web/stubs/gridDebugLogger.ts"),
      [path.resolve("src/processing/gridVisualizer.ts")]: path.resolve("web/stubs/gridVisualizer.ts"),
    },
  },
};
```

## Risks & open questions
- **PDF.js rendering parity**: PDF.js's canvas renderer is used by Firefox's built-in PDF viewer, so it's battle-tested, but visual quality can differ from PDFium on edge cases (unusual fonts, complex gradients). Acceptable for OCR input and user-facing page previews; document the swap in README.
- **Top-level `await importPdfJs()`** in `pdfjs.ts:57`: this runs at module-eval time. Browsers support TLA in ES modules, so fine, but Vite dev server + worker spawning interplay needs a smoke test.
- **Bundle size**: PDF.js + Tesseract WASM will be a few MB total. Acceptable. Lazy-load Tesseract on first OCR run to keep first-paint fast. (PDFium dropped — net bundle win.)
- **CORS on Tesseract CDN**: works from `file://`? No — `tesseract.js` fetches from jsdelivr which should have permissive CORS, but `file://` origins can't do fetch. User must serve via `vite dev` or a static server. Document this: `npm run dev:web` is the intended entry, not double-clicking `index.html`.
- **Type mismatches**: `Buffer` appears in engine interfaces (`renderPageImage` returns `Buffer`, OCR accepts `string | Buffer`). In the browser we return `Uint8Array`. Keep the `Buffer` type and polyfill via a small Vite-injected `Buffer` shim — avoids changing public types.
- **Image-bounds loss**: without PDFium's `_FPDFPageObj_GetBounds`, the browser build reports no embedded-image bounds. Consequence: the "targeted OCR around images" path is disabled; full-page OCR kicks in for text-sparse pages. This matches common usage but is worth noting.

## Testing: Playwright + red/green TDD

The web app is tested **end-to-end with Playwright, driven by strict red/green TDD**. Every feature lands as: write a failing test → watch it fail for the right reason → write the minimum code to pass → confirm green → refactor. No code is written without a failing test pointing at it first.

Unit tests are out of scope for the web layer — the parser core already has vitest unit coverage, and the interesting surface for the web app is the browser behavior (file picker, toggles, parser running in the actual browser runtime with real WASM). Playwright exercises that surface directly.

### Setup
- Add `@playwright/test` to devDependencies.
- `playwright.config.ts`: point at `http://localhost:5173` (Vite dev), run `vite` via `webServer` config so `npm run test:web` boots the dev server automatically.
- Tests under `web/tests/`. One spec per feature.
- Test fixtures under `web/tests/fixtures/`: a small text-based PDF (`sample-text.pdf`) and a small scanned/image-based PDF (`sample-scanned.pdf`). Keep each under ~50 KB — copy the smallest PDFs we can find from existing test corpus if `test/` or `fixtures/` has any; otherwise generate with `@pdf-lib/pdf-lib` at test-setup time.
- Use Playwright's `page.setInputFiles()` to drive the `<input type="file">` — works headlessly.

### Test list (written in this order, each red before green)

Each item below is a single `test(...)` block. Write → fail → implement → pass → commit before moving to the next.

1. **`loads the app shell`**
   - Navigate to `/`. Assert title, file input, OCR checkbox, screenshots checkbox, parse button, both `<textarea>`s (`#text-output`, `#json-output`), and both Copy buttons are visible.
   - Red driver: no `index.html` yet.
   - Minimum green: write `index.html` with the required controls.

2. **`parse button is disabled until a file is picked`**
   - Assert button `disabled` on load. Pick `sample-text.pdf`. Assert button enabled.
   - Red driver: wiring in `main.ts` doesn't exist.
   - Minimum green: add the disabled-state logic.

3. **`rejects non-PDF files with a clear error`**
   - Pick a `.txt` file. Click parse. Assert error message containing "PDF" in `#status`; both textareas stay empty.
   - Red driver: no file validation.
   - Minimum green: PDF magic-number check before calling `LiteParse`.

4. **`parses a text PDF with OCR off and renders both text and JSON`**
   - OCR checkbox unchecked. Pick `sample-text.pdf`. Click parse.
   - Wait for a known substring from the fixture PDF to appear in `#text-output`'s `value`.
   - Assert `#json-output`'s `value` is valid JSON (`JSON.parse` succeeds), contains a `pages` array, and is multi-line (pretty-printed).
   - Red driver: parser not wired. This is the walking skeleton that forces Vite aliases, stubs, the `LiteParseBrowser` wrapper, and the dual-pane UI all to work.
   - Minimum green: implement steps 1–7 of "Step-by-step execution" until this test passes.

5. **`copy buttons copy the respective textarea to the clipboard`**
   - Grant `clipboard-read` / `clipboard-write` permissions in Playwright context.
   - After test 4's parse, click the Text pane's Copy button, read clipboard, assert matches `#text-output.value`.
   - Click the JSON pane's Copy button, read clipboard, assert matches `#json-output.value`.
   - Red driver: Copy handlers not wired.
   - Minimum green: `navigator.clipboard.writeText` on each Copy button click.

6. **`shows a status indicator while parsing`**
   - Click parse, immediately assert a "Parsing…" / busy indicator is visible. Wait for completion, assert it's gone.
   - Red driver: no status UI.
   - Minimum green: toggle `#status` text around the parse call.

7. **`parses a scanned PDF with OCR on and extracts text via Tesseract`**
   - OCR checkbox checked. Pick `sample-scanned.pdf`. Click parse. Wait (long timeout — Tesseract first run fetches traineddata) for known text from the image to appear in `#text-output`.
   - Red driver: Tesseract + PDF.js canvas-render path not proven in the browser until this runs. Likely flushes out Vite asset and Worker config issues.
   - Minimum green: wire `web/pdfjs-renderer.ts` so `renderPageToImageData` returns ImageData into Tesseract; confirm Tesseract.js Web Workers initialize; fix any Vite asset-loading issues that come up.
   - Use `test.slow()`; first run may take 30–60 s on CI fetching traineddata.

8. **`OCR toggle actually changes behavior`**
   - Parse the scanned PDF with OCR off. Assert `#text-output.value.length` is below a small threshold.
   - Parse same PDF with OCR on. Assert length exceeds a much larger threshold and contains expected text.
   - Red driver: if the toggle is accidentally ignored, both runs produce the same output.
   - Minimum green: already green if test 7 is right; this is a regression guard.

9. **`surfaces parser errors to the user`**
   - Pick a corrupted PDF (a file that starts with `%PDF-` but has garbage after). Click parse. Assert error text appears in `#status` with an `error` class and the parse button is re-enabled.
   - Red driver: error handling likely throws unhandled into console.
   - Minimum green: `try/catch` around the parse call.

10. **`reparsing replaces prior output`**
    - Parse one PDF. Parse a different PDF. Assert prior text is gone from both textareas and new content is present in both.
    - Red driver: UI might append instead of replace.
    - Minimum green: clear both textareas at parse start.

11. **`renders page screenshots when requested`**
    - Check the "render screenshots" checkbox. Parse `sample-text.pdf`. Wait for `#screenshots img` to appear. Assert at least one image, non-zero `naturalWidth`/`naturalHeight`, and `src` starts with `blob:`.
    - Red driver: screenshots path not wired; `LiteParse.screenshot()` in the browser bundle won't return anything until the PDF.js canvas renderer's `convertToBlob` path is in place.
    - Minimum green: implement `renderPageToBuffer` in `web/pdfjs-renderer.ts` via `OffscreenCanvas.convertToBlob({ type: "image/png" })`; wire the `#shots` checkbox in `main.ts`.
    - Second assertion: unchecking the box and re-parsing removes prior screenshots (guards the `URL.revokeObjectURL` cleanup).

12. **`mobile viewport: controls stack vertically and stay usable`**
    - `test.use({ viewport: { width: 375, height: 812 } })` (iPhone-ish).
    - Load the app. Assert the two result panes stack vertically (the bounding box of `#json-output` is below `#text-output`, not beside it).
    - Assert parse button bounding height ≥ 40px (touch target).
    - Run test 4's flow on mobile viewport; assert both textareas still populate and Copy buttons still work.
    - Red driver: responsive CSS not in place.
    - Minimum green: finish the `@media (min-width: 900px)` breakpoint + touch-sized controls.

### TDD discipline
- **One failing test at a time.** Don't batch red tests.
- **Watch it fail first.** Every test must be seen red with an error message that matches the intended failure reason before any implementation. Screenshot the red failure (Playwright's trace viewer is enough) — don't trust "it probably failed."
- **Minimum green.** Write the least code needed to get the current test green, even if it's obviously incomplete. Let the next test drive the next increment.
- **Refactor on green.** Once a test is green, refactor freely — as long as the test stays green. Don't refactor while red.
- **No code without a test.** If a piece of code isn't reached by any Playwright test, either write a test for it or delete it. Stubs and Vite aliases are exempt (they're build config, not behavior).
- **Commit per test.** Each test + its implementation = one commit. Makes the trail reviewable and makes bisects cheap.

### CI
Add a `test:web` npm script: `playwright test`. Run in CI on PRs. `playwright install --with-deps chromium` in the CI setup step. Keep the browser matrix to chromium only for v1 — no Firefox/WebKit until the feature is stable.

### Trace capture
Enable `trace: "retain-on-failure"` in `playwright.config.ts`. WASM + Workers in a browser produce obscure errors; traces with the console log + network tab are essential for diagnosing first-run failures.

## Step-by-step execution (TDD-driven)

Numbered to match the test list above. "Write test N → red → implement → green."

0. **Scaffold** (pre-TDD, one-time): install `vite`, `@playwright/test`; create `playwright.config.ts`, empty `web/index.html`, `web/style.css`, empty `vite.config.ts`, test fixtures.
1. Test 1 → `index.html` shell with both textareas + copy buttons + screenshots checkbox.
2. Test 2 → `main.ts` file-input wiring (disabled/enabled state).
3. Test 3 → PDF magic-number validation.
4. Test 4 → **the big one.** To go green:
   - `web/stubs/*.ts` — empty shims for Node modules (fs, path, os, child_process, axios, form-data, file-type).
   - `web/pdfjsImporter.ts` — browser-safe PDF.js loader with `workerSrc` set.
   - `web/pdfjs-renderer.ts` — full replacement for `pdfium-renderer.ts` using PDF.js's canvas renderer.
   - Vite aliases wired (see Section 10).
   - `web/liteparse-browser.ts` — thin wrapper around `LiteParse`.
   - Edit `src/engines/ocr/tesseract.ts` — guard `process.env`.
   - `main.ts` — read file → arrayBuffer → parse (force `outputFormat: "json"`) → render text into `#text-output` and pretty-JSON into `#json-output`.
5. Test 5 → Copy button handlers via `navigator.clipboard.writeText`.
6. Test 6 → status UI toggle around the parse call.
7. Test 7 → Tesseract + PDF.js canvas render end-to-end. Likely the hardest integration moment.
8. Test 8 → regression guard (usually green already).
9. Test 9 → error handling: try/catch, `#status` class, button re-enable.
10. Test 10 → clear both textareas + revoke screenshot URLs at parse start.
11. Test 11 → wire `#shots` checkbox + `renderPageToBuffer` via `OffscreenCanvas.convertToBlob`.
12. Test 12 → responsive CSS: single-column stack at narrow widths, two-column grid ≥ 900px, 44px min touch targets, viewport meta tag.

Only after all 12 are green: add `npm run dev:web` / `build:web` / `test:web` scripts to `package.json`, update README with browser usage notes.

## Time estimate (revised with TDD)
- Scaffold + tests 1–3 (shell + validation): ½ day.
- Test 4 (walking skeleton, most integration pain): 1 day.
- Tests 5–6 (copy buttons + status): ¼ day.
- Test 7 (OCR): 1 day, mostly debugging Tesseract Web Worker + Vite asset loading.
- Tests 8–10 (hardening): ½ day.
- Test 11 (screenshots): ¼ day — the encode helper is ~10 lines.
- Test 12 (responsive CSS): ¼ day.

Total: ~3.5–4 days. Slower than the no-TDD estimate, but each step lands with a regression guard and the walking-skeleton test (test 4) catches 80% of integration problems at a point where they're cheap to diagnose.
