// Browser version of pdfjsImporter.
// No Node fs/url shims needed — the browser has DOMMatrix, Path2D, ImageData natively.
// Vite will rewrite the `new URL(..., import.meta.url)` references into bundled asset URLs.

// Safari < 17 / older iOS WebKit ship ReadableStream but not its
// [Symbol.asyncIterator]. PDF.js does `for await (const v of readableStream)`
// internally while streaming text content; without this polyfill the first
// parse throws "undefined is not a function (near '...value of readableStream...')".
// Install this BEFORE importing pdf.mjs so any early code paths see it.
if (
  typeof ReadableStream !== "undefined" &&
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  !(Symbol.asyncIterator in (ReadableStream.prototype as any))
) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (ReadableStream.prototype as any)[Symbol.asyncIterator] = async function* iterator() {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const reader = (this as any).getReader();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return;
        yield value;
      }
    } finally {
      reader.releaseLock();
    }
  };
}

// @ts-expect-error vendored ESM build has no types
import * as pdfjs from "../src/vendor/pdfjs/pdf.mjs";

// Wire the PDF.js worker URL. Vite resolves this to a hashed asset URL at build time.
const workerUrl = new URL("../src/vendor/pdfjs/pdf.worker.mjs", import.meta.url).href;
(pdfjs as { GlobalWorkerOptions: { workerSrc: string } }).GlobalWorkerOptions.workerSrc = workerUrl;

export async function importPdfJs() {
  return {
    fn: (pdfjs as { getDocument: (opts: unknown) => unknown }).getDocument,
    // Base URL for cmaps/standard_fonts/wasm assets — we staged them under /pdfjs/ in public/.
    // Prefixed with Vite's BASE_URL so it resolves correctly when deployed to a sub-path
    // (e.g. GitHub Pages project sites like /liteparse/).
    dir: `${import.meta.env.BASE_URL}pdfjs`,
  };
}
