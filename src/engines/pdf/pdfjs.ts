import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import {
  PdfEngine,
  PdfDocument,
  PageData,
  Path,
  Image,
  Annotation,
} from './interface.js';
import { TextItem } from '../../core/types.js';
import { PdfiumRenderer } from './pdfium-renderer.js';

// Dynamic import of PDF.js
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
// From dist/src/engines/pdf/ we need to go up to dist/src/vendor/pdfjs
const PDFJS_DIR = join(__dirname, '../../vendor/pdfjs');

// Import PDF.js dynamically
await import(`${PDFJS_DIR}/pdf.mjs`);
const pdfjs = await import(`${PDFJS_DIR}/pdf.mjs`);
const { getDocument } = pdfjs;

const CMAP_URL = `${PDFJS_DIR}/cmaps/`;
const STANDARD_FONT_DATA_URL = `${PDFJS_DIR}/standard_fonts/`;
const CMAP_PACKED = true;

/**
 * Extract rotation angle in degrees from PDF transformation matrix
 * Matrix format: [a, b, c, d, e, f] where rotation is atan2(b, a)
 */
function getRotation(transform: number[]): number {
  return Math.atan2(transform[1], transform[0]) * (180 / Math.PI);
}

/**
 * Multiply two transformation matrices
 */
function multiplyMatrices(m1: number[], m2: number[]): number[] {
  return [
    m1[0] * m2[0] + m1[2] * m2[1],
    m1[1] * m2[0] + m1[3] * m2[1],
    m1[0] * m2[2] + m1[2] * m2[3],
    m1[1] * m2[2] + m1[3] * m2[3],
    m1[0] * m2[4] + m1[2] * m2[5] + m1[4],
    m1[1] * m2[4] + m1[3] * m2[5] + m1[5]
  ];
}

/**
 * Apply transformation matrix to a point
 */
function applyTransformation(point: { x: number; y: number }, transform: number[]): { x: number; y: number } {
  return {
    x: point.x * transform[0] + point.y * transform[2] + transform[4],
    y: point.x * transform[1] + point.y * transform[3] + transform[5]
  };
}

/**
 * Decompose transformation matrix to get scale factors using SVD
 * This computes the singular values of the 2D transformation matrix
 */
function singularValueDecompose2dScale(m: number[]): { x: number; y: number } {
  // Create transpose of the 2x2 part of the matrix
  const transpose = [m[0], m[2], m[1], m[3]];

  // Multiply matrix m with its transpose to get eigenvalues
  const a = m[0] * transpose[0] + m[1] * transpose[2];
  const b = m[0] * transpose[1] + m[1] * transpose[3];
  const c = m[2] * transpose[0] + m[3] * transpose[2];
  const d = m[2] * transpose[1] + m[3] * transpose[3];

  // Solve the second degree polynomial to get roots (eigenvalues)
  const first = (a + d) / 2;
  const second = Math.sqrt((a + d) ** 2 - 4 * (a * d - c * b)) / 2;
  const sx = first + second || 1;
  const sy = first - second || 1;

  // Scale values are the square roots of the eigenvalues
  return { x: Math.sqrt(sx), y: Math.sqrt(sy) };
}

export class PdfJsEngine implements PdfEngine {
  name = 'pdfjs';
  private pdfiumRenderer: PdfiumRenderer = new PdfiumRenderer();
  private currentPdfPath: string | null = null;

  async loadDocument(filePath: string): Promise<PdfDocument> {
    const data = new Uint8Array(await fs.readFile(filePath));

    // Store path for PDFium rendering
    this.currentPdfPath = filePath;

    const loadingTask = getDocument({
      data,
      cMapUrl: CMAP_URL,
      cMapPacked: CMAP_PACKED,
      standardFontDataUrl: STANDARD_FONT_DATA_URL,
    });

    const pdfDocument = await loadingTask.promise;
    const metadata = await pdfDocument.getMetadata();

    return {
      numPages: pdfDocument.numPages,
      data,
      metadata,
      _pdfDocument: pdfDocument,
    } as PdfDocument & { _pdfDocument: any };
  }

  async extractPage(doc: PdfDocument, pageNum: number): Promise<PageData> {
    const pdfDocument = (doc as any)._pdfDocument;
    const page = await pdfDocument.getPage(pageNum);

    // Get viewport
    const viewport = page.getViewport({ scale: 1.0 });

    // Extract text content
    const textContent = await page.getTextContent();
    const textItems: TextItem[] = textContent.items
      .filter((item: any) => item.height !== 0 && item.width !== 0)
      .map((item: any) => {
        // Apply viewport transformation to convert PDF coordinates to screen coordinates
        // This properly handles Y-axis flip (PDF is bottom-up, screen is top-down)
        const cm = multiplyMatrices(viewport.transform, item.transform);

        // Get lower-left corner (text space origin)
        const ll = applyTransformation({ x: 0, y: 0 }, cm);

        // Get scale factors to properly size the bounding box
        const scale = singularValueDecompose2dScale(item.transform);

        // Get upper-right corner
        const ur = applyTransformation(
          { x: item.width / scale.x, y: item.height / scale.y },
          cm
        );

        // Calculate final bounding box in viewport space
        const left = Math.min(ll.x, ur.x);
        const right = Math.max(ll.x, ur.x);
        const top = Math.min(ll.y, ur.y);
        const bottom = Math.max(ll.y, ur.y);

        const width = right - left;
        const height = bottom - top;

        // Calculate rotation from combined transformation matrix
        let rotation = getRotation(cm);
        // Normalize to 0-360 range
        if (rotation < 0) {
          rotation += 360;
        }

        // Decode buggy font markers from PDF.js
        // Format: :->|>_<charCode>_<fontChar>_<|<-:
        let decodedStr = item.str.replace(
          /:->|>_(\d+)_\d+_<|<-:/g,
          (_: string, charCode: string) => String.fromCharCode(parseInt(charCode))
        );

        // Handle pipe-separated characters: " |a|  |r|  |X| " -> "arX"
        // Some PDFs encode text with characters separated by pipes and spaces
        if (decodedStr.includes('|')) {
          const pipePattern = /\s*\|([^|])\|\s*/g;
          const matches = [...decodedStr.matchAll(pipePattern)];
          if (matches.length > 0) {
            decodedStr = matches.map(m => m[1]).join('');
          }
        }

        return {
          str: decodedStr,
          x: left,
          y: top,
          width,
          height,
          w: width,
          h: height,
          r: rotation,
          fontName: item.fontName,
          fontSize: Math.sqrt(
            item.transform[0] * item.transform[0] + item.transform[1] * item.transform[1]
          ),
        };
      })
      .filter((item: TextItem) => {
        // Filter out items that are off-page (negative coordinates or beyond page bounds)
        return (
          item.y >= 0 &&
          item.x >= 0 &&
          item.y <= viewport.height &&
          item.x <= viewport.width
        );
      });

    // Extract annotations (links, etc.)
    const rawAnnotations = await page.getAnnotations();
    const annotations: Annotation[] = rawAnnotations.map((ann: any) => ({
      type: ann.annotationType,
      subtype: ann.subtype,
      url: ann.url,
      rect: ann.rect,
    }));

    // For now, paths and images are empty - we'll add extraction logic later
    const paths: Path[] = [];
    const images: Image[] = [];

    await page.cleanup();

    return {
      pageNum,
      width: viewport.width,
      height: viewport.height,
      textItems,
      paths,
      images,
      annotations,
    };
  }

  async extractAllPages(
    doc: PdfDocument,
    maxPages?: number,
    targetPages?: string
  ): Promise<PageData[]> {
    const numPages = Math.min(
      doc.numPages,
      maxPages || doc.numPages
    );

    const pages: PageData[] = [];

    // Parse target pages if specified
    let pageNumbers: number[] = [];
    if (targetPages) {
      pageNumbers = this.parseTargetPages(targetPages, numPages);
    } else {
      pageNumbers = Array.from({ length: numPages }, (_, i) => i + 1);
    }

    for (const pageNum of pageNumbers) {
      if (maxPages && pages.length >= maxPages) {
        break;
      }
      const pageData = await this.extractPage(doc, pageNum);
      pages.push(pageData);
    }

    return pages;
  }

  async renderPageImage(
    _doc: PdfDocument,
    pageNum: number,
    dpi: number
  ): Promise<Buffer> {
    // Use PDFium for rendering (more robust with inline images)
    if (!this.currentPdfPath) {
      throw new Error('PDF path not available for rendering');
    }

    return await this.pdfiumRenderer.renderPageToBuffer(
      this.currentPdfPath,
      pageNum,
      dpi
    );
  }

  async close(doc: PdfDocument): Promise<void> {
    const pdfDocument = (doc as any)._pdfDocument;
    if (pdfDocument && pdfDocument.destroy) {
      await pdfDocument.destroy();
    }

    // Clean up PDFium renderer
    await this.pdfiumRenderer.close();
    this.currentPdfPath = null;
  }

  private parseTargetPages(targetPages: string, maxPages: number): number[] {
    const pages: number[] = [];
    const parts = targetPages.split(',');

    for (const part of parts) {
      const trimmed = part.trim();
      if (trimmed.includes('-')) {
        // Range: "1-5"
        const [start, end] = trimmed.split('-').map((n) => parseInt(n.trim()));
        for (let i = start; i <= Math.min(end, maxPages); i++) {
          if (i >= 1) {
            pages.push(i);
          }
        }
      } else {
        // Single page: "10"
        const pageNum = parseInt(trimmed);
        if (pageNum >= 1 && pageNum <= maxPages) {
          pages.push(pageNum);
        }
      }
    }

    return [...new Set(pages)].sort((a, b) => a - b);
  }
}
