import { PDFiumLibrary, type PDFiumPageRenderOptions } from "@hyzyla/pdfium";
import sharp from "sharp";
import { promises as fs } from "fs";

/**
 * Minimal interface for PDFium's WASM module internals.
 * These properties are private/protected in @hyzyla/pdfium's typings,
 * but we need direct access for low-level page object bound queries.
 */
interface PdfiumWasmModule {
  _FPDFPageObj_GetBounds?: (
    objHandle: number,
    leftPtr: number,
    bottomPtr: number,
    rightPtr: number,
    topPtr: number
  ) => number;
  _FPDF_GetPageWidthF: (pagePtr: number) => number;
  _FPDF_GetPageHeightF: (pagePtr: number) => number;
  _malloc: (size: number) => number;
  _free: (ptr: number) => void;
  HEAPU8: Uint8Array;
}

/** Minimum image dimension in PDF points to be considered for OCR */
const MIN_IMAGE_SIZE_PT = 25;
/** Images covering more than this fraction of the page are treated as backgrounds */
const MAX_IMAGE_PAGE_COVERAGE = 0.9;

interface PdfiumPageInternal {
  module: PdfiumWasmModule;
  pageIdx: number;
  objects(): Iterable<{ type: string; objectIdx: number }>;
}

/**
 * PDFium-based PDF screenshot renderer
 * Uses native PDFium library for high-quality, fast screenshots
 */
export class PdfiumRenderer {
  private pdfium: PDFiumLibrary | null = null;

  async init(): Promise<void> {
    if (!this.pdfium) {
      this.pdfium = await PDFiumLibrary.init();
    }
  }

  async renderPageToBuffer(
    pdfInput: string | Buffer | Uint8Array,
    pageNumber: number,
    dpi: number = 150,
    password?: string
  ): Promise<Buffer> {
    await this.init();

    if (!this.pdfium) {
      throw new Error("PDFium not initialized");
    }

    // Accept file path or raw bytes
    const pdfBuffer =
      typeof pdfInput === "string" ? await fs.readFile(pdfInput) : Buffer.from(pdfInput);

    // Load document
    const document = await this.pdfium.loadDocument(pdfBuffer, password);

    try {
      // Get page (0-indexed in pdfium)
      const page = document.getPage(pageNumber - 1);

      // Calculate scale from DPI (72 DPI is the default)
      const scale = dpi / 72;

      // Render page using Sharp for image processing
      const image = await page.render({
        scale,
        render: async (options: PDFiumPageRenderOptions) => {
          return await sharp(options.data, {
            raw: {
              width: options.width,
              height: options.height,
              channels: 4, // RGBA
            },
          })
            .png({
              compressionLevel: 6,
            })
            .withMetadata({
              density: dpi,
            })
            .toBuffer();
        },
      });

      return Buffer.from(image.data);
    } finally {
      // Clean up document
      document.destroy();
    }
  }

  /**
   * Extract bounding boxes of all embedded images on a page.
   * Uses PDFium's low-level WASM API to iterate page objects and read image bounds.
   * Returns coordinates in viewport space (Y-down, origin top-left) in PDF points.
   */
  async extractImageBounds(
    pdfInput: string | Buffer | Uint8Array,
    pageNumber: number,
    password?: string
  ): Promise<Array<{ x: number; y: number; width: number; height: number }>> {
    await this.init();

    if (!this.pdfium) {
      throw new Error("PDFium not initialized");
    }

    const pdfBuffer =
      typeof pdfInput === "string" ? await fs.readFile(pdfInput) : Buffer.from(pdfInput);

    const document = await this.pdfium.loadDocument(pdfBuffer, password);

    try {
      // Cast to internal interface — module/pageIdx/objectIdx are private in
      // @hyzyla/pdfium typings but required for low-level WASM bound queries
      const page = document.getPage(pageNumber - 1) as unknown as PdfiumPageInternal;
      const results: Array<{ x: number; y: number; width: number; height: number }> = [];

      const mod = page.module;
      const pagePtr = page.pageIdx;

      if (!mod || !mod._FPDFPageObj_GetBounds) {
        return results;
      }

      // Get page dimensions via WASM (getSize() is private in typings)
      const pageWidth = mod._FPDF_GetPageWidthF(pagePtr);
      const pageHeight = mod._FPDF_GetPageHeightF(pagePtr);

      // Iterate page objects looking for images
      for (const obj of page.objects()) {
        if (obj.type !== "image") continue;

        const objHandle = obj.objectIdx;
        if (!objHandle) continue;

        // Allocate memory for 4 floats (left, bottom, right, top)
        const ptr = mod._malloc(16);
        try {
          const ok = mod._FPDFPageObj_GetBounds(objHandle, ptr, ptr + 4, ptr + 8, ptr + 12);
          if (!ok) continue;

          const buf = mod.HEAPU8.buffer;
          const view = new DataView(buf);
          const left = view.getFloat32(ptr, true);
          const bottom = view.getFloat32(ptr + 4, true);
          const right = view.getFloat32(ptr + 8, true);
          const top = view.getFloat32(ptr + 12, true);

          const w = right - left;
          const h = top - bottom;

          if (w < MIN_IMAGE_SIZE_PT || h < MIN_IMAGE_SIZE_PT) continue;
          if (w > pageWidth * MAX_IMAGE_PAGE_COVERAGE && h > pageHeight * MAX_IMAGE_PAGE_COVERAGE)
            continue;

          // Convert PDF coords (Y-up) to viewport coords (Y-down)
          results.push({
            x: left,
            y: pageHeight - top,
            width: w,
            height: h,
          });
        } finally {
          mod._free(ptr);
        }
      }

      return results;
    } finally {
      document.destroy();
    }
  }

  async close(): Promise<void> {
    // PDFium WASM doesn't need explicit cleanup
    if (this.pdfium) {
      this.pdfium.destroy();
      this.pdfium = null;
    }
  }
}
