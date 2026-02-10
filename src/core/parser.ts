import { LiteParseConfig, ParseResult, ScreenshotResult } from './types.js';
import { mergeConfig } from './config.js';
import { PdfEngine } from '../engines/pdf/interface.js';
import { PdfJsEngine } from '../engines/pdf/pdfjs.js';
import { OcrEngine } from '../engines/ocr/interface.js';
import { TesseractEngine } from '../engines/ocr/tesseract.js';
import { HttpOcrEngine } from '../engines/ocr/http-simple.js';
import { projectPagesToGrid } from '../processing/grid.js';
import { buildBoundingBoxes } from '../processing/bbox.js';
import { detectTables } from '../processing/tables.js';
import { formatJSON } from '../output/json.js';
import {
  convertToPdf,
  cleanupConversionFiles,
} from '../conversion/convertToPdf.js';

export class LiteParse {
  private config: LiteParseConfig;
  private pdfEngine: PdfEngine;
  private ocrEngine?: OcrEngine;

  constructor(userConfig: Partial<LiteParseConfig> = {}) {
    // Merge user config with defaults
    this.config = mergeConfig(userConfig);

    // Initialize PDF engine
    this.pdfEngine = new PdfJsEngine()

    // Initialize OCR engine
    // Auto-detect: use HTTP OCR if URL provided, otherwise use Tesseract
    if (this.config.ocrEnabled) {
      if (this.config.ocrServerUrl) {
        this.ocrEngine = new HttpOcrEngine(this.config.ocrServerUrl);
      } else {
        this.ocrEngine = new TesseractEngine();
      }
    }
  }

  /**
   * Parse a PDF file and return structured result
   */
  async parse(filePath: string, quiet = false): Promise<ParseResult> {
    const log = (msg: string) => {
      if (!quiet) console.error(msg); // Progress goes to stderr
    };

    // Convert to PDF if needed
    let pdfPath = filePath;
    let needsCleanup = false;

    log(`Processing file: ${filePath}`);
    const conversionResult = await convertToPdf(filePath);

    if ('code' in conversionResult) {
      // Conversion error
      throw new Error(`Conversion failed: ${conversionResult.message}`);
    }

    pdfPath = conversionResult.pdfPath;
    needsCleanup = pdfPath !== filePath;

    if (needsCleanup) {
      log(`Converted ${conversionResult.originalExtension} to PDF`);
    }

    log(`Parsing PDF: ${pdfPath}`);
    log(`Using ${this.pdfEngine.name} engine`);

    // Load PDF document
    const doc = await this.pdfEngine.loadDocument(pdfPath);
    log(`Loaded PDF with ${doc.numPages} pages`);

    // Extract pages
    const pages = await this.pdfEngine.extractAllPages(
      doc,
      this.config.maxPages,
      this.config.targetPages
    );
    log(`Extracted ${pages.length} pages`);

    // Process pages with complete grid projection
    const processedPages = projectPagesToGrid(pages, this.config);

    // Build bounding boxes if enabled
    if (this.config.preciseBoundingBox) {
      for (const page of processedPages) {
        page.boundingBoxes = buildBoundingBoxes(page.textItems);
      }
    }

    // Detect tables if enabled
    if (this.config.tableDetection) {
      for (let i = 0; i < processedPages.length; i++) {
        const page = processedPages[i];
        const tables = detectTables(pages[i].paths, pages[i].textItems);
        page.tables = tables;
      }
    }

    // OCR integration
    if (this.ocrEngine) {
      await this.runOCR(doc, pages, processedPages, log);
    }

    // Build final text
    const fullText = processedPages.map((p) => p.text).join('\n\n');

    // Close PDF document
    await this.pdfEngine.close(doc);

    // Cleanup OCR engine if it's Tesseract (to free memory)
    if (this.ocrEngine && 'terminate' in this.ocrEngine) {
      await (this.ocrEngine as TesseractEngine).terminate();
    }

    // Cleanup temporary conversion files
    if (needsCleanup) {
      await cleanupConversionFiles(pdfPath);
    }

    const result: ParseResult = {
      pages: processedPages,
      text: fullText,
    };

    // Format based on output format
    switch (this.config.outputFormat) {
      case 'json':
        result.json = JSON.parse(formatJSON(result));
        break;
      case 'text':
        // Already in text format
        break;
    }

    return result;
  }

  /**
   * Generate screenshots of PDF pages
   */
  async screenshot(
    filePath: string,
    pageNumbers?: number[],
    quiet = false
  ): Promise<ScreenshotResult[]> {
    const log = (msg: string) => {
      if (!quiet) console.error(msg); // Progress goes to stderr
    };

    log(`Generating screenshots for: ${filePath}`);

    // Load PDF document
    const doc = await this.pdfEngine.loadDocument(filePath);

    const results: ScreenshotResult[] = [];
    const pages = pageNumbers || Array.from({ length: doc.numPages }, (_, i) => i + 1);

    for (const pageNum of pages) {
      if (pageNum < 1 || pageNum > doc.numPages) {
        console.error(`Skipping invalid page number: ${pageNum}`);
        continue;
      }

      log(`Rendering page ${pageNum}...`);
      const imageBuffer = await this.pdfEngine.renderPageImage(
        doc,
        pageNum,
        this.config.dpi
      );

      // Get page dimensions
      const pageData = await this.pdfEngine.extractPage(doc, pageNum);

      results.push({
        pageNum,
        width: pageData.width,
        height: pageData.height,
        imageBuffer,
      });
    }

    // Close PDF document
    await this.pdfEngine.close(doc);

    log(`Generated ${results.length} screenshots`);
    return results;
  }

  /**
   * Run OCR on pages that need it
   */
  private async runOCR(
    doc: any,
    originalPages: any[],
    processedPages: any[],
    log: (msg: string) => void
  ): Promise<void> {
    if (!this.ocrEngine) return;

    log('Running OCR on pages...');

    for (let i = 0; i < originalPages.length; i++) {
      const page = originalPages[i];
      const processedPage = processedPages[i];

      // Check if page has very little text (indicating need for OCR)
      const textLength = page.textItems.reduce(
        (sum: number, item: any) => sum + item.str.length,
        0
      );

      // Run OCR if text is sparse or if images exist that might contain text
      if (textLength < 100 || page.images.length > 0) {
        try {
          // Render page as image
          const imageBuffer = await this.pdfEngine.renderPageImage(
            doc,
            page.pageNum,
            this.config.dpi
          );

          // Save temporary image file
          const fs = await import('fs/promises');
          const path = await import('path');
          const os = await import('os');
          const tmpDir = os.tmpdir();
          const tmpImagePath = path.join(
            tmpDir,
            `page_${page.pageNum}_ocr.png`
          );
          await fs.writeFile(tmpImagePath, imageBuffer);

          // Run OCR
          log(`  OCR on page ${page.pageNum}...`);
          const ocrResults = await this.ocrEngine.recognize(tmpImagePath, {
            language: this.config.ocrLanguage,
            correctRotation: true,
          });

          // Convert OCR results to text items and add to processed page
          if (ocrResults.length > 0) {
            const ocrTextItems = ocrResults
              .filter((r) => r.confidence > 0.1) // Filter low confidence
              .map((r) => ({
                str: r.text,
                x: r.bbox[0],
                y: r.bbox[1],
                width: r.bbox[2] - r.bbox[0],
                height: r.bbox[3] - r.bbox[1],
                fontName: 'OCR',
                fontSize: r.bbox[3] - r.bbox[1],
                fromOcr: true,
              }));

            // Add OCR text to page (will be merged in grid projection if needed)
            processedPage.ocrTextItems = ocrTextItems;
            log(
              `  Found ${ocrTextItems.length} text items from OCR on page ${page.pageNum}`
            );
          }

          // Clean up temp file
          await fs.unlink(tmpImagePath).catch(() => {});
        } catch (error) {
          log(`  OCR failed for page ${page.pageNum}: ${error}`);
        }
      }
    }
  }

  /**
   * Get current configuration
   */
  getConfig(): LiteParseConfig {
    return { ...this.config };
  }
}
