// Browser stub for PdfiumRenderer.
// A real browser app would replace this with a PDF.js canvas renderer.
// This stub is enough to verify the bundle builds.
export class PdfiumRenderer {
  async init() {}
  async loadDocument() {}
  closeDocument() {}
  async renderPageToBuffer() { return new Uint8Array(0); }
  async extractImageBounds() { return []; }
  async close() {}
}
