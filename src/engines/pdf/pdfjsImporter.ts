import { fileURLToPath } from "node:url";
import { dirname } from "node:path";

// PDF.js 5.x requires DOMMatrix/Path2D/ImageData which aren't available in Node.js.
// We only use PDF.js for text extraction (not rendering), so minimal stubs suffice.
const g = globalThis as Record<string, unknown>;
if (typeof g.DOMMatrix === "undefined") {
  g.DOMMatrix = class DOMMatrix {
    a = 1;
    b = 0;
    c = 0;
    d = 1;
    e = 0;
    f = 0;
    constructor(init?: number[]) {
      if (init && init.length >= 6) {
        [this.a, this.b, this.c, this.d, this.e, this.f] = init;
      }
    }
  };
}
if (typeof g.Path2D === "undefined") {
  g.Path2D = class Path2D {};
}
if (typeof g.ImageData === "undefined") {
  g.ImageData = class ImageData {
    width = 0;
    height = 0;
    data = new Uint8ClampedArray(0);
    constructor(w?: number, h?: number) {
      this.width = w ?? 0;
      this.height = h ?? 0;
    }
  };
}

export async function importPdfJs() {
  const pdfUrl = new URL("../../vendor/pdfjs/pdf.mjs", import.meta.url);
  const pdfjs = await import(pdfUrl.href);

  const dirPath = dirname(fileURLToPath(pdfUrl));
  return {
    fn: pdfjs.getDocument,
    dir: dirPath,
  };
}
