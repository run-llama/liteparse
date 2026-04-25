---
title: Browser Usage
description: Run LiteParse in the browser with Vite.
sidebar:
  order: 6
---

LiteParse's core parsing engine — PDF.js text extraction, spatial grid projection, and OCR via Tesseract.js — can run entirely in the browser. Since the library includes Node-only dependencies (sharp, fs, child_process, etc.), you need a bundler like [Vite](https://vite.dev/) to swap those out with browser-safe stubs at build time.

## What works in the browser

- **PDF parsing** from `Uint8Array` input (use `file.arrayBuffer()` to get bytes from a file picker)
- **OCR** via Tesseract.js (runs in Web Workers, fetches language data from CDN on first use)
- **Text and JSON output formats**

## What doesn't work

- **File path input** — pass `Uint8Array` instead
- **DOCX/XLSX/PPTX/image conversion** — requires LibreOffice/ImageMagick which aren't available in the browser
- **HTTP OCR server** — irrelevant for client-side use; Tesseract.js runs directly in the browser
- **Screenshots** — these use PDFium + sharp, which are native Node addons

## Vite setup

The setup requires two things:

1. **A Vite plugin** that redirects Node-only source files to browser-safe stub replacements
2. **`resolve.alias` entries** that stub out Node built-in modules (`fs`, `path`, `child_process`, etc.)

### Full Vite config

```typescript
// vite.config.ts
import { defineConfig, type Plugin } from "vite";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = (p: string) =>
  resolve(fileURLToPath(new URL(".", import.meta.url)), p);

// These Node-only files need browser replacements.
// Point each target at your own stub file (see below).
const FILE_REDIRECTS: Array<{ match: RegExp; target: string }> = [
  {
    match: /\/engines\/pdf\/pdfium-renderer(\.js|\.ts)?$/,
    target: here("stubs/pdfium-renderer.ts"),
  },
  {
    match: /\/engines\/pdf\/pdfjsImporter(\.js|\.ts)?$/,
    target: here("stubs/pdfjsImporter.ts"),
  },
  {
    match: /\/engines\/ocr\/http-simple(\.js|\.ts)?$/,
    target: here("stubs/http-simple.ts"),
  },
  {
    match: /\/conversion\/convertToPdf(\.js|\.ts)?$/,
    target: here("stubs/convertToPdf.ts"),
  },
  {
    match: /\/processing\/gridDebugLogger(\.js|\.ts)?$/,
    target: here("stubs/gridDebugLogger.ts"),
  },
  {
    match: /\/processing\/gridVisualizer(\.js|\.ts)?$/,
    target: here("stubs/gridVisualizer.ts"),
  },
];

function liteparseNodeRedirects(): Plugin {
  return {
    name: "liteparse-node-redirects",
    enforce: "pre",
    async resolveId(source, importer) {
      if (!importer) return null;
      const importerDir = dirname(importer);
      const absolutePath = source.startsWith(".")
        ? resolve(importerDir, source)
        : source;
      for (const { match, target } of FILE_REDIRECTS) {
        if (match.test(absolutePath) || match.test(source)) {
          return target;
        }
      }
      return null;
    },
  };
}

export default defineConfig({
  plugins: [liteparseNodeRedirects()],
  optimizeDeps: {
    include: ["tesseract.js"],
  },
  resolve: {
    alias: [
      { find: "node:fs/promises", replacement: here("stubs/empty.ts") },
      { find: "node:fs", replacement: here("stubs/empty.ts") },
      { find: "node:url", replacement: here("stubs/empty.ts") },
      { find: "node:path", replacement: here("stubs/empty.ts") },
      { find: "node:os", replacement: here("stubs/empty.ts") },
      { find: "node:child_process", replacement: here("stubs/empty.ts") },
      { find: /^fs$/, replacement: here("stubs/empty.ts") },
      { find: /^fs\/promises$/, replacement: here("stubs/empty.ts") },
      { find: /^path$/, replacement: here("stubs/empty.ts") },
      { find: /^os$/, replacement: here("stubs/empty.ts") },
      { find: /^child_process$/, replacement: here("stubs/empty.ts") },
      { find: "form-data", replacement: here("stubs/empty.ts") },
      { find: "axios", replacement: here("stubs/empty.ts") },
      { find: "file-type", replacement: here("stubs/file-type.ts") },
    ],
  },
  define: {
    "process.env.NODE_ENV": JSON.stringify("production"),
  },
});
```

### Stub files

You need to create stub files that export the same interfaces as the Node-only modules but with browser-safe (or no-op) implementations. A complete working example with all stubs is available in the [`scripts/browser-compat/`](https://github.com/run-llama/liteparse/tree/main/scripts/browser-compat) directory.

The key stubs:

**`stubs/empty.ts`** — catch-all for Node built-ins:
```typescript
export default {};
```

**`stubs/convertToPdf.ts`** — only PDF magic-byte detection is needed:
```typescript
export async function convertToPdf() {
  throw new Error("File conversion is not supported in browser environments.");
}
export async function convertBufferToPdf() {
  throw new Error("File conversion is not supported in browser environments.");
}
export async function cleanupConversionFiles() {}
export async function guessExtensionFromBuffer(data: Uint8Array) {
  if (data[0] === 0x25 && data[1] === 0x50 && data[2] === 0x44 && data[3] === 0x46) {
    return ".pdf";
  }
  return null;
}
```

**`stubs/pdfjsImporter.ts`** — browser-safe PDF.js loader (no `node:url`/`node:path`):
```typescript
// @ts-expect-error vendored ESM build has no types
import * as pdfjs from "@llamaindex/liteparse/vendor/pdfjs/pdf.mjs";

export async function importPdfJs() {
  return {
    fn: (pdfjs as any).getDocument,
    dir: new URL("@llamaindex/liteparse/vendor/pdfjs", import.meta.url).href,
  };
}
```

**`stubs/file-type.ts`** — minimal file type detection:
```typescript
export async function fileTypeFromBuffer(buf: Uint8Array) {
  if (buf[0] === 0x25 && buf[1] === 0x50 && buf[2] === 0x44 && buf[3] === 0x46) {
    return { ext: "pdf", mime: "application/pdf" };
  }
  return undefined;
}
export async function fileTypeFromFile() {
  return undefined;
}
```

The remaining stubs (`pdfium-renderer`, `http-simple`, `gridDebugLogger`, `gridVisualizer`) just need to export the same class/function names with no-op implementations. See the full example in `scripts/browser-compat/stubs/`.

## Usage in your app

Once the bundler is configured, use LiteParse the same way as in Node — just pass `Uint8Array` instead of file paths:

```typescript
import { LiteParse } from "@llamaindex/liteparse";

const parser = new LiteParse({
  ocrEnabled: true,
  outputFormat: "json",
});

// From a file picker
const fileInput = document.querySelector<HTMLInputElement>("#file")!;
fileInput.addEventListener("change", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;

  const bytes = new Uint8Array(await file.arrayBuffer());
  const result = await parser.parse(bytes);

  console.log(result.text);
  console.log(JSON.stringify(result.json, null, 2));
});
```

## Known gotchas

- **First OCR run is slow.** Tesseract.js fetches `eng.traineddata` (~10 MB) from the jsdelivr CDN on first use. Subsequent runs use the browser cache.
- **`file://` doesn't work.** The PDF.js worker and WASM assets need to be served over HTTP. Use `vite dev` or a static server.
- **Safari < 17.** Older Safari/iOS WebKit ships `ReadableStream` without `Symbol.asyncIterator`, which PDF.js needs. You may need a polyfill — see the [MDN docs](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStream#browser_compatibility).
- **ArrayBuffer detachment.** PDF.js transfers the input buffer to its worker, which detaches the original `ArrayBuffer`. If you need the bytes again after parsing (e.g., for screenshots), read the file a second time with `file.arrayBuffer()`.
- **Bundle size.** Expect ~500 KB for the app bundle plus ~2.3 MB for the PDF.js worker. Tesseract loads lazily on first OCR run.
