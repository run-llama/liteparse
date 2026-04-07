import pLimit from "p-limit";
import {
  LiteParseConfig,
  LiteParseInput,
  ParseResult,
  ScreenshotResult,
  TextItem,
} from "./types.js";
import { mergeConfig } from "./config.js";
import { PdfEngine, PdfDocument, PageData } from "../engines/pdf/interface.js";
import { PdfJsEngine } from "../engines/pdf/pdfjs.js";
import { PdfiumRenderer } from "../engines/pdf/pdfium-renderer.js";
import { OcrEngine } from "../engines/ocr/interface.js";
import { TesseractEngine } from "../engines/ocr/tesseract.js";
import { HttpOcrEngine } from "../engines/ocr/http-simple.js";
import { projectPagesToGrid } from "../processing/grid.js";
import { buildBoundingBoxes } from "../processing/bbox.js";
import { formatJSON } from "../output/json.js";
import {
  convertToPdf,
  convertBufferToPdf,
  cleanupConversionFiles,
  guessExtensionFromBuffer,
} from "../conversion/convertToPdf.js";
import { cleanOcrTableArtifacts } from "../processing/textUtils.js";

/**
 * Main document parser class. Handles PDF parsing, OCR, format conversion,
 * and screenshot generation.
 *
 * @example Basic text extraction
 * ```typescript
 * import { LiteParse } from "@llamaindex/liteparse";
 *
 * const parser = new LiteParse();
 * const result = await parser.parse("document.pdf");
 * console.log(result.text);
 * ```
 *
 * @example JSON output with bounding boxes
 * ```typescript
 * const parser = new LiteParse({ outputFormat: "json", dpi: 300 });
 * const result = await parser.parse("document.pdf");
 * for (const page of result.json.pages) {
 *   console.log(`Page ${page.page}: ${page.boundingBoxes.length} bounding boxes`);
 * }
 * ```
 *
 * @example Using an HTTP OCR server
 * ```typescript
 * const parser = new LiteParse({
 *   ocrServerUrl: "http://localhost:8828/ocr",
 *   ocrLanguage: "en",
 * });
 * const result = await parser.parse("scanned-document.pdf");
 * ```
 */
export class LiteParse {
  private config: LiteParseConfig;
  private pdfEngine: PdfEngine;
  private ocrEngine?: OcrEngine;

  /**
   * Create a new LiteParse instance.
   *
   * @param userConfig - Partial configuration to override defaults. See {@link LiteParseConfig} for all options.
   */
  constructor(userConfig: Partial<LiteParseConfig> = {}) {
    // Merge user config with defaults
    this.config = mergeConfig(userConfig);

    // Initialize PDF engine
    this.pdfEngine = new PdfJsEngine();

    // Initialize OCR engine
    // Auto-detect: use HTTP OCR if URL provided, otherwise use Tesseract
    if (this.config.ocrEnabled) {
      if (this.config.ocrServerUrl) {
        this.ocrEngine = new HttpOcrEngine(this.config.ocrServerUrl);
      } else {
        this.ocrEngine = new TesseractEngine(this.config.numWorkers, this.config.tessdataPath);
      }
    }
  }

  /**
   * Parse a document and return the extracted text, page data, and optionally structured JSON.
   *
   * Supports PDFs natively. Non-PDF formats (DOCX, XLSX, images, etc.) are automatically
   * converted to PDF before parsing if the required system tools are installed.
   *
   * @param input - A file path, `Buffer`, or `Uint8Array` containing document bytes.
   *   When given raw bytes, PDF data is parsed directly with zero disk I/O.
   *   Non-PDF bytes are written to a temp file for format conversion.
   * @param quiet - If `true`, suppresses progress logging to stderr.
   * @returns Parsed document data including text, per-page info, and optional JSON.
   *
   * @throws Error if the file cannot be found, converted, or parsed.
   */
  async parse(input: LiteParseInput, quiet = false): Promise<ParseResult> {
    const log = (msg: string) => {
      if (!quiet) console.error(msg); // Progress goes to stderr
    };

    let doc: PdfDocument;
    let needsCleanup = false;
    let cleanupPath: string | undefined;

    if (typeof input === "string") {
      log(`Processing file: ${input}`);
      const conversionResult = await convertToPdf(input, this.config.password);

      if ("code" in conversionResult) {
        throw new Error(`Conversion failed: ${conversionResult.message}`);
      }

      if ("content" in conversionResult) {
        log(`File is a text-based format. Returning content directly.`);
        return { pages: [], text: conversionResult.content };
      }

      const pdfPath = conversionResult.pdfPath;
      needsCleanup = pdfPath !== input;
      if (needsCleanup) {
        cleanupPath = pdfPath;
        log(`Converted ${conversionResult.originalExtension} to PDF`);
      }

      doc = await this.pdfEngine.loadDocument(pdfPath, this.config.password);
    } else {
      log(`Processing buffer input (${input.byteLength} bytes)`);
      const ext = await guessExtensionFromBuffer(input);

      if (ext === ".pdf") {
        // Zero-disk path: pass bytes directly to the PDF engine
        const data = input instanceof Uint8Array ? input : new Uint8Array(input);
        doc = await this.pdfEngine.loadDocument(data, this.config.password);
      } else {
        // Non-PDF buffer: write to temp file for conversion
        const conversionResult = await convertBufferToPdf(input, this.config.password);

        if ("code" in conversionResult) {
          throw new Error(`Conversion failed: ${conversionResult.message}`);
        }

        if ("content" in conversionResult) {
          log(`Buffer is a text-based format. Returning content directly.`);
          return { pages: [], text: conversionResult.content };
        }

        needsCleanup = true;
        cleanupPath = conversionResult.pdfPath;
        log(`Converted ${conversionResult.originalExtension} buffer to PDF`);
        doc = await this.pdfEngine.loadDocument(conversionResult.pdfPath, this.config.password);
      }
    }

    log(`Loaded PDF with ${doc.numPages} pages`);

    try {
      // Extract pages
      const pages = await this.pdfEngine.extractAllPages(
        doc,
        this.config.maxPages,
        this.config.targetPages,
        { extractImages: this.config.ocrEnabled }
      );

      // run BEFORE grid projection
      if (this.ocrEngine) {
        await this.runOCR(doc, pages, log);
      }

      // Process pages with complete grid projection (after OCR)
      const processedPages = await projectPagesToGrid(pages, this.config);

      // Build bounding boxes if enabled
      if (this.config.preciseBoundingBox) {
        for (const page of processedPages) {
          page.boundingBoxes = buildBoundingBoxes(page.textItems);
        }
      }

      // Build final text
      const fullText = processedPages.map((p) => p.text).join("\n\n");

      const result: ParseResult = {
        pages: processedPages,
        text: fullText,
      };

      // Format based on output format
      switch (this.config.outputFormat) {
        case "json":
          result.json = JSON.parse(formatJSON(result));
          break;
        case "text":
          // Already in text format
          break;
      }

      return result;
    } finally {
      // Always release resources, even if processing throws
      await this.pdfEngine.close(doc);

      if (this.ocrEngine && "terminate" in this.ocrEngine) {
        await (this.ocrEngine as TesseractEngine).terminate();
      }

      if (needsCleanup && cleanupPath) {
        await cleanupConversionFiles(cleanupPath);
      }
    }
  }

  /**
   * Generate screenshots of PDF pages as image buffers.
   *
   * Uses PDFium for high-quality rendering. Each page is returned as a
   * {@link ScreenshotResult} with the raw image buffer and dimensions.
   *
   * Supports PDFs natively. Non-PDF formats (DOCX, XLSX, images, etc.) are automatically
   * converted to PDF before rendering if the required system tools are installed.
   * Text-based formats (TXT, CSV, etc.) cannot be screenshotted and will throw an error.
   *
   * @param input - A file path, `Buffer`, or `Uint8Array` containing document bytes.
   * @param pageNumbers - 1-indexed page numbers to screenshot. If omitted, all pages are rendered.
   * @param quiet - If `true`, suppresses progress logging to stderr.
   * @returns Array of screenshot results, one per rendered page.
   *
   * @throws Error if the input is a text-based format that cannot be rendered.
   * @throws Error if the file cannot be found, converted, or rendered.
   */
  async screenshot(
    input: LiteParseInput,
    pageNumbers?: number[],
    quiet = false
  ): Promise<ScreenshotResult[]> {
    const log = (msg: string) => {
      if (!quiet) console.error(msg); // Progress goes to stderr
    };

    log(`Generating screenshots for: ${typeof input === "string" ? input : "<buffer>"}`);

    let doc: PdfDocument;
    let pdfInput: string | Uint8Array;
    let needsCleanup = false;
    let cleanupPath: string | undefined;

    if (typeof input === "string") {
      const conversionResult = await convertToPdf(input, this.config.password);

      if ("code" in conversionResult) {
        throw new Error(`Conversion failed: ${conversionResult.message}`);
      }

      if ("content" in conversionResult) {
        throw new Error(`Cannot screenshot text-based format. Convert to PDF first.`);
      }

      const pdfPath = conversionResult.pdfPath;
      needsCleanup = pdfPath !== input;
      if (needsCleanup) {
        cleanupPath = pdfPath;
        log(`Converted ${conversionResult.originalExtension} to PDF`);
      }

      doc = await this.pdfEngine.loadDocument(pdfPath, this.config.password);
      pdfInput = pdfPath;
    } else {
      const ext = await guessExtensionFromBuffer(input);

      if (ext === ".pdf") {
        const data = input instanceof Uint8Array ? input : new Uint8Array(input);
        doc = await this.pdfEngine.loadDocument(data, this.config.password);
        pdfInput = data;
      } else {
        const conversionResult = await convertBufferToPdf(input, this.config.password);

        if ("code" in conversionResult) {
          throw new Error(`Conversion failed: ${conversionResult.message}`);
        }

        if ("content" in conversionResult) {
          throw new Error(`Cannot screenshot text-based format. Convert to PDF first.`);
        }

        needsCleanup = true;
        cleanupPath = conversionResult.pdfPath;
        log(`Converted ${conversionResult.originalExtension} buffer to PDF`);
        doc = await this.pdfEngine.loadDocument(conversionResult.pdfPath, this.config.password);
        pdfInput = conversionResult.pdfPath;
      }
    }

    const totalPages = doc.numPages;
    const results: ScreenshotResult[] = [];
    const pages = pageNumbers || Array.from({ length: totalPages }, (_, i) => i + 1);

    const renderer = new PdfiumRenderer();
    await renderer.loadDocument(pdfInput, this.config.password);

    try {
      for (const pageNum of pages) {
        if (pageNum < 1 || pageNum > totalPages) {
          console.error(`Skipping invalid page number: ${pageNum}`);
          continue;
        }

        log(`Rendering page ${pageNum}...`);
        const imageBuffer = await renderer.renderPageToBuffer(pdfInput, pageNum, this.config.dpi);

        const pageData = await this.pdfEngine.extractPage(doc, pageNum, { extractImages: false });

        results.push({
          pageNum,
          width: pageData.width,
          height: pageData.height,
          imageBuffer,
        });
      }
    } finally {
      await renderer.close();
      await this.pdfEngine.close(doc);

      if (needsCleanup && cleanupPath) {
        await cleanupConversionFiles(cleanupPath);
      }
    }

    log(`Generated ${results.length} screenshots`);
    return results;
  }

  /**
   * Run OCR on pages that need it (in parallel with concurrency limit)
   */
  private async runOCR(
    doc: PdfDocument,
    pages: PageData[],
    log: (msg: string) => void
  ): Promise<void> {
    if (!this.ocrEngine) return;

    log(`Running OCR on pages (concurrency: ${this.config.numWorkers})...`);

    const limit = pLimit(this.config.numWorkers);

    await Promise.all(pages.map((page) => limit(() => this.processPageOcr(doc, page, log))));
  }

  /**
   * Process OCR for a single page
   */
  private async processPageOcr(
    doc: PdfDocument,
    page: PageData,
    log: (msg: string) => void
  ): Promise<void> {
    if (!this.ocrEngine) return;

    // Check if page has very little text (indicating need for OCR)
    const textLength = page.textItems.reduce(
      (sum: number, item: TextItem) => sum + item.str.length,
      0
    );

    // Determine if OCR is needed and what mode
    const hasGarbledRegions = page.garbledTextRegions && page.garbledTextRegions.length > 0;
    const needsFullOcr = textLength < 100 || page.images.length > 0;

    if (!needsFullOcr && !hasGarbledRegions) {
      return;
    }

    try {
      // Render page as image buffer
      const imageBuffer = await this.pdfEngine.renderPageImage(
        doc,
        page.pageNum,
        this.config.dpi,
        this.config.password
      );

      // Run OCR directly on the buffer (no temp file needed)
      log(`  OCR on page ${page.pageNum}...`);
      const ocrResults = await this.ocrEngine.recognize(imageBuffer, {
        language: this.config.ocrLanguage,
        correctRotation: true,
      });

      // Convert OCR results to text items and add to page
      if (ocrResults.length > 0) {
        // Scale factor to convert from OCR pixels to PDF points
        // OCR operates at config.dpi, PDF uses 72 points per inch (PDF spec constant)
        const scaleFactor = 72 / this.config.dpi;

        // Helper to check if an OCR result overlaps with garbled regions
        const overlapsGarbledRegion = (ocrBbox: number[]): boolean => {
          if (!page.garbledTextRegions) return false;

          const ocrX = ocrBbox[0] * scaleFactor;
          const ocrY = ocrBbox[1] * scaleFactor;
          const ocrW = (ocrBbox[2] - ocrBbox[0]) * scaleFactor;
          const ocrH = (ocrBbox[3] - ocrBbox[1]) * scaleFactor;

          // Check overlap with any garbled region (with some tolerance)
          const tolerance = 5; // PDF points
          for (const region of page.garbledTextRegions) {
            const overlapX =
              ocrX < region.x + region.width + tolerance && ocrX + ocrW > region.x - tolerance;
            const overlapY =
              ocrY < region.y + region.height + tolerance && ocrY + ocrH > region.y - tolerance;
            if (overlapX && overlapY) {
              return true;
            }
          }
          return false;
        };

        // Helper to check if an OCR result spatially overlaps with existing PDF text
        // This prevents duplicating text that PDF already extracted correctly
        const overlapsExistingText = (ocrBbox: number[]): boolean => {
          const ocrX = ocrBbox[0] * scaleFactor;
          const ocrY = ocrBbox[1] * scaleFactor;
          const ocrW = (ocrBbox[2] - ocrBbox[0]) * scaleFactor;
          const ocrH = (ocrBbox[3] - ocrBbox[1]) * scaleFactor;

          const tolerance = 2; // PDF points - tighter tolerance for existing text
          for (const item of page.textItems) {
            const itemRight = item.x + (item.width || item.w || 0);
            const itemBottom = item.y + (item.height || item.h || 0);

            const overlapX = ocrX < itemRight + tolerance && ocrX + ocrW > item.x - tolerance;
            const overlapY = ocrY < itemBottom + tolerance && ocrY + ocrH > item.y - tolerance;

            if (overlapX && overlapY) {
              return true;
            }
          }
          return false;
        };

        const ocrTextItems: TextItem[] = ocrResults
          .filter((r) => r.confidence > 0.1) // Filter low confidence
          .filter((r) => {
            // For targeted OCR (garbled regions only), only include results that overlap
            if (hasGarbledRegions && !needsFullOcr) {
              return overlapsGarbledRegion(r.bbox);
            }
            // For full OCR, include all results
            return true;
          })
          .filter((r) => {
            // Skip OCR results that spatially overlap with existing PDF text
            // This prevents duplicating text that PDF already extracted correctly
            return !overlapsExistingText(r.bbox);
          })
          .map((r) => {
            // Clean OCR artifacts from table border misreads
            const cleanedText = cleanOcrTableArtifacts(r.text);
            return {
              str: cleanedText,
              x: r.bbox[0] * scaleFactor,
              y: r.bbox[1] * scaleFactor,
              width: (r.bbox[2] - r.bbox[0]) * scaleFactor,
              height: (r.bbox[3] - r.bbox[1]) * scaleFactor,
              w: (r.bbox[2] - r.bbox[0]) * scaleFactor,
              h: (r.bbox[3] - r.bbox[1]) * scaleFactor,
              fontName: "OCR",
              fontSize: (r.bbox[3] - r.bbox[1]) * scaleFactor,
              confidence: Math.round(r.confidence * 1000) / 1000,
            };
          })
          .filter((item) => item.str.length > 0); // Skip items that became empty after cleaning

        // Add OCR text items directly to page textItems
        page.textItems.push(...ocrTextItems);
        log(`  Found ${ocrTextItems.length} text items from OCR on page ${page.pageNum}`);
      }
    } catch (error) {
      log(`  OCR failed for page ${page.pageNum}: ${error}`);
    }
  }

  /**
   * Get a copy of the current configuration, including defaults merged with user overrides.
   *
   * @returns A shallow copy of the active {@link LiteParseConfig}.
   */
  getConfig(): LiteParseConfig {
    return { ...this.config };
  }
}
