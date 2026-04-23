// Browser version of pdfjsImporter.
// No Node fs/url shims needed — the browser has DOMMatrix, Path2D, ImageData natively.
// Vite will rewrite the `new URL(..., import.meta.url)` references into bundled asset URLs.

// @ts-expect-error vendored ESM build has no types
import * as pdfjs from "../src/vendor/pdfjs/pdf.mjs";

// Wire the PDF.js worker URL. Vite resolves this to a hashed asset URL at build time.
const workerUrl = new URL("../src/vendor/pdfjs/pdf.worker.mjs", import.meta.url).href;
(pdfjs as { GlobalWorkerOptions: { workerSrc: string } }).GlobalWorkerOptions.workerSrc = workerUrl;

export async function importPdfJs() {
  return {
    fn: (pdfjs as { getDocument: (opts: unknown) => unknown }).getDocument,
    // Base URL for cmaps/standard_fonts/wasm assets — we staged them under /pdfjs/ in public/.
    dir: "/pdfjs",
  };
}
