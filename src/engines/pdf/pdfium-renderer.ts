import { PDFiumLibrary, type PDFiumPageRenderOptions } from "@hyzyla/pdfium";
import sharp from "sharp";
import { promises as fs } from "fs";

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

  async close(): Promise<void> {
    // PDFium WASM doesn't need explicit cleanup
    if (this.pdfium) {
      this.pdfium.destroy();
      this.pdfium = null;
    }
  }
}
