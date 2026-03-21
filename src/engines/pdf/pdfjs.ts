import fs from "node:fs/promises";
import { PdfEngine, PdfDocument, PageData, Image, Annotation, BoundingBox } from "./interface.js";
import { TextItem } from "../../core/types.js";
import { PdfiumRenderer } from "./pdfium-renderer.js";
import { importPdfJs } from "./pdfjsImporter.js";

/** PDF.js internal document type - opaque to our code */
interface PdfJsDocument {
  numPages: number;
  getPage(pageNum: number): Promise<PdfJsPage>;
  getMetadata(): Promise<unknown>;
  destroy(): Promise<void>;
}

/** PDF.js internal page type */
interface PdfJsPage {
  getViewport(params: { scale: number }): PdfJsViewport;
  getTextContent(): Promise<PdfJsTextContent>;
  cleanup(): Promise<void>;
}

/** PDF.js viewport type */
interface PdfJsViewport {
  width: number;
  height: number;
  transform: number[];
}

/** PDF.js text content type */
interface PdfJsTextContent {
  items: PdfJsTextItem[];
}

/** PDF.js text item type */
interface PdfJsTextItem {
  str: string;
  transform: number[];
  width: number;
  height: number;
  fontName?: string;
}

/** Extended PdfDocument with internal PDF.js document reference */
interface PdfJsExtendedDocument extends PdfDocument {
  _pdfDocument: PdfJsDocument;
}

// Dynamic import of PDF.js
const { fn: getDocument, dir: PDFJS_DIR } = await importPdfJs();

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
    m1[1] * m2[4] + m1[3] * m2[5] + m1[5],
  ];
}

/**
 * Apply transformation matrix to a point
 */
function applyTransformation(
  point: { x: number; y: number },
  transform: number[]
): { x: number; y: number } {
  return {
    x: point.x * transform[0] + point.y * transform[2] + transform[4],
    y: point.x * transform[1] + point.y * transform[3] + transform[5],
  };
}

// Pre-compiled regex patterns for string decoding
const BUGGY_FONT_MARKER_REGEX = /:->\|>_(\d+)_\d+_<\|<-:/g;
const BUGGY_FONT_MARKER_CHECK = ":->|>";
const PIPE_PATTERN_REGEX = /\s*\|([^|])\|\s*/g;

/**
 * Common tabular figures font encoding mappings.
 * Many fonts with "Differences" arrays use similar patterns for tabular digits.
 * These mappings are derived from common font encoding conventions.
 *
 * Note: The same PDF can use multiple fonts with DIFFERENT glyph-to-character mappings
 * for the same glyph IDs. We try all mappings and pick the best match.
 *
 * Special glyphs:
 * - 42: '*' (asterisk for significance markers)
 * - 150: '-' (minus sign/dash)
 */
const TABULAR_FIGURES_MAPPINGS: Record<number, string>[] = [
  // Mapping 1: Bold/header style (e.g., census PDF header row)
  // Characters: 0123456789.,
  {
    17: "4",
    18: "6",
    19: "8",
    20: "5",
    21: "9",
    22: "7",
    23: "1",
    24: " ",
    25: ",",
    26: "+",
    27: "-",
    28: "3",
    29: "0",
    30: "2",
    31: ".",
    42: "*",
    150: "-",
  },
  // Mapping 2: Book/body style (e.g., census PDF detail rows)
  // Note: Same glyph IDs but different character assignments!
  {
    17: "+",
    18: "7",
    19: "-",
    20: "9",
    21: "6",
    22: "3",
    23: "1",
    24: " ",
    25: "8",
    26: "5",
    27: "4",
    28: "0",
    29: "2",
    30: ".",
    31: ",",
    42: "*",
    150: "-",
  },
];

/**
 * Check if all glyphs in the range would produce printable ASCII via direct char code.
 * Returns true if using String.fromCharCode on these glyphs would produce valid text.
 */
function canDecodeAsAscii(glyphs: number[]): boolean {
  // Check if ALL glyphs would produce valid printable ASCII or common whitespace
  for (const g of glyphs) {
    // Printable ASCII range (space through tilde), plus tab/newline
    if (!((g >= 32 && g <= 126) || g === 9 || g === 10 || g === 13)) {
      return false;
    }
  }
  return true;
}

/**
 * Score a decoded string for how "number-like" it appears.
 * Higher scores indicate better number formatting.
 */
function scoreNumberFormat(decoded: string): number {
  let score = 0;

  // Count digits - primary indicator of a number
  const digitCount = (decoded.match(/[0-9]/g) || []).length;
  score += digitCount * 2;

  // Bonus for matching common number patterns
  // Pattern: digits with optional commas for thousands
  if (/^\d{1,3}(,\d{3})*$/.test(decoded)) {
    score += 5; // e.g., "248,800"
  }
  // Pattern: decimal number
  if (/^\d+\.\d+$/.test(decoded)) {
    score += 5; // e.g., "10.5"
  }
  // Pattern: negative number
  if (/^[*-]?\d/.test(decoded)) {
    score += 2; // e.g., "-1,132" or "*-0.4"
  }
  // Pattern: percentage or simple number
  if (/^\d+$/.test(decoded)) {
    score += 3; // e.g., "897"
  }

  // Penalize bad patterns
  // Consecutive punctuation marks (not valid in numbers)
  if (/[.,]{2,}/.test(decoded)) {
    score -= 10;
  }
  // Punctuation at start (except minus/asterisk) or end
  if (/^[.,+]|[.,+]$/.test(decoded)) {
    score -= 5;
  }
  // Comma followed by anything other than 3 digits then boundary
  if (/,(?!\d{3}(?:[,.]|$))/.test(decoded)) {
    score -= 3;
  }
  // Period not followed by digits (except at end)
  if (/\.(?![0-9])/.test(decoded) && !decoded.endsWith(".")) {
    score -= 3;
  }

  return score;
}

/**
 * Try to decode buggy font markers using known tabular figures mappings.
 * Returns the decoded string if a mapping produces valid-looking text,
 * otherwise returns null to fall back to charCode decoding.
 *
 * Strategy:
 * 1. If glyphs are in ASCII range (32-126), let the fallback handle it
 * 2. If glyphs are in tabular range (17-31, plus special chars), try mappings
 * 3. Score each result for how "number-like" it appears
 * 4. Return the best result if it looks like a valid number
 */
function tryDecodeTabularFigures(str: string): string | null {
  if (!str.includes(BUGGY_FONT_MARKER_CHECK)) return null;

  // Extract all glyph IDs from the markers
  const glyphs: number[] = [];
  let match;
  const regex = /:->\|>_(\d+)_\d+_<\|<-:/g;
  while ((match = regex.exec(str)) !== null) {
    glyphs.push(parseInt(match[1]));
  }

  if (glyphs.length === 0) return null;

  // If these glyphs would decode fine as ASCII, don't use tabular mapping
  if (canDecodeAsAscii(glyphs)) {
    return null;
  }

  // Check if glyphs are in the tabular figures range
  // Tabular figures typically use glyphs 17-31, plus special chars like 42, 150
  const tabularRange = glyphs.every(
    (g) =>
      (g >= 17 && g <= 31) || // Core tabular figures
      g === 42 || // Asterisk
      g === 150 || // Minus
      g === 8 ||
      g === 9 ||
      g === 10 // Some special chars
  );

  if (!tabularRange) {
    // Mixed content - not pure tabular figures
    return null;
  }

  // Try each mapping and pick the best result
  let bestResult: string | null = null;
  let bestScore = -Infinity;

  for (const mapping of TABULAR_FIGURES_MAPPINGS) {
    const decoded = glyphs.map((g) => mapping[g] || "").join("");

    // Skip if there are unmapped glyphs
    const unmapped = glyphs.filter((g) => !mapping[g]).length;
    if (unmapped > 0) continue;

    // Score based on how "number-like" the result looks
    const score = scoreNumberFormat(decoded);

    if (score > bestScore) {
      bestScore = score;
      bestResult = decoded;
    }
  }

  // Only return if we got a reasonable score (at least some digits, proper format)
  if (bestResult && bestScore > 0) {
    return bestResult;
  }

  return null;
}

/**
 * Strip C0/C1 control characters from text (except common whitespace).
 * These can appear in PDF text due to font encoding issues but the
 * surrounding text may still be valid.
 */
function stripControlChars(str: string): string {
  let result = "";
  for (const char of str) {
    const code = char.charCodeAt(0);
    // Skip C0 controls (except tab, newline, carriage return) and C1 controls
    if (
      (code >= 0x00 && code <= 0x1f && code !== 0x09 && code !== 0x0a && code !== 0x0d) ||
      (code >= 0x80 && code <= 0x9f)
    ) {
      continue;
    }
    result += char;
  }
  return result;
}

/**
 * Detect garbled text from fonts with corrupted ToUnicode mappings.
 *
 * When PDF fonts lack proper ToUnicode maps, PDF.js may output characters
 * mapped to unexpected Unicode code points. Common patterns include:
 *
 * 1. Private Use Area (PUA) characters - fonts often map glyphs here
 * 2. Mix of unrelated scripts (Arabic + Latin Extended in English text)
 * 3. Rare/obscure Unicode blocks appearing in normal text
 * 4. Control characters (when text is predominantly control chars)
 *
 * Returns true if the string appears to be garbled font output.
 */
function isGarbledFontOutput(str: string): boolean {
  if (str.length < 3) return false;

  let privateUseCount = 0;
  let arabicCount = 0;
  let latinExtendedCount = 0;
  let basicLatinLetterCount = 0;
  let suspiciousCount = 0; // Other suspicious Unicode ranges
  let controlCharCount = 0; // C0/C1 control characters
  let normalCharCount = 0; // Normal printable characters

  for (const char of str) {
    const code = char.charCodeAt(0);

    // C0 control characters (0x00-0x1F) except common whitespace (tab, newline, carriage return)
    // C1 control characters (0x80-0x9F)
    if (
      (code >= 0x00 && code <= 0x1f && code !== 0x09 && code !== 0x0a && code !== 0x0d) ||
      (code >= 0x80 && code <= 0x9f)
    ) {
      controlCharCount++;
    }
    // Private Use Area (U+E000-U+F8FF) - almost always garbled
    else if (code >= 0xe000 && code <= 0xf8ff) {
      privateUseCount++;
    }
    // Arabic block (0x600-0x6FF) and Arabic Extended (0x750-0x77F, 0x8A0-0x8FF)
    else if (
      (code >= 0x600 && code <= 0x6ff) ||
      (code >= 0x750 && code <= 0x77f) ||
      (code >= 0x8a0 && code <= 0x8ff)
    ) {
      arabicCount++;
    }
    // Latin Extended-A (0x100-0x17F), Latin Extended-B (0x180-0x24F),
    // Latin Extended Additional (0x1E00-0x1EFF)
    else if ((code >= 0x100 && code <= 0x24f) || (code >= 0x1e00 && code <= 0x1eff)) {
      latinExtendedCount++;
    }
    // Basic Latin letters (a-z, A-Z)
    else if ((code >= 0x41 && code <= 0x5a) || (code >= 0x61 && code <= 0x7a)) {
      basicLatinLetterCount++;
      normalCharCount++;
    }
    // Suspicious ranges that rarely appear in normal text:
    // - Syriac (0x700-0x74F)
    // - Thaana (0x780-0x7BF)
    // - NKo (0x7C0-0x7FF)
    // - Samaritan (0x800-0x83F)
    // - Specials (0xFFF0-0xFFFF)
    // - Geometric Shapes (0x25A0-0x25FF) in running text
    // - Box Drawing (0x2500-0x257F) in running text
    // - Combining Diacritical Marks alone (0x0300-0x036F)
    else if (
      (code >= 0x700 && code <= 0x7ff) || // Syriac, Thaana, NKo
      (code >= 0x800 && code <= 0x83f) || // Samaritan
      (code >= 0xfff0 && code <= 0xffff) || // Specials
      (code >= 0x2500 && code <= 0x25ff) || // Box drawing, geometric shapes
      (code >= 0x0300 && code <= 0x036f) // Combining marks (suspicious if frequent)
    ) {
      suspiciousCount++;
    }
    // Normal printable characters (digits, punctuation, common symbols, space)
    else if ((code >= 0x20 && code <= 0x7e) || code === 0x09 || code === 0x0a || code === 0x0d) {
      normalCharCount++;
    }
  }

  const totalChars = str.length;

  // Text is predominantly control characters - definitely garbled
  // This catches cases like more_hard_2.pdf where text is entirely control chars
  if (controlCharCount > 0 && controlCharCount > normalCharCount) {
    return true;
  }

  // Private Use Area characters are almost always garbled fonts
  if (privateUseCount >= 2) {
    return true;
  }

  // Mix of Arabic AND Latin Extended is extremely rare in legitimate text
  if (arabicCount >= 2 && latinExtendedCount >= 2) {
    return true;
  }

  // High concentration of suspicious characters
  if (suspiciousCount >= 3 || suspiciousCount > totalChars * 0.2) {
    return true;
  }

  // Text predominantly Latin Extended with very few basic Latin letters
  // (legitimate Latin-script text would have mostly basic Latin)
  if (latinExtendedCount > totalChars * 0.3 && basicLatinLetterCount < totalChars * 0.2) {
    return true;
  }

  // Mix of Arabic/suspicious with Latin Extended (script mixing)
  if ((arabicCount >= 1 || suspiciousCount >= 1) && latinExtendedCount >= 3) {
    return true;
  }

  return false;
}

export class PdfJsEngine implements PdfEngine {
  name = "pdfjs";
  private pdfiumRenderer: PdfiumRenderer | null = null;
  private currentPdfPath: string | null = null;
  private currentPdfData: Uint8Array | null = null;

  async loadDocument(input: string | Uint8Array, password?: string): Promise<PdfDocument> {
    let data: Uint8Array;
    if (typeof input === "string") {
      data = new Uint8Array(await fs.readFile(input));
      this.currentPdfPath = input;
    } else {
      // pdf.js requires a plain Uint8Array, not a Buffer subclass
      data = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      this.currentPdfPath = null;
    }

    // Store data for buffer-based rendering
    this.currentPdfData = data;

    const loadingTask = getDocument({
      data,
      password,
      cMapUrl: CMAP_URL,
      cMapPacked: CMAP_PACKED,
      standardFontDataUrl: STANDARD_FONT_DATA_URL,
    });

    let pdfDocument: PdfJsDocument;
    try {
      pdfDocument = await loadingTask.promise;
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("password") || message.includes("Password")) {
        if (password) {
          throw new Error("Incorrect password for this PDF. Please check the password and try again.");
        } else {
          throw new Error("This PDF is password-protected. Use --password <password> to provide the document password.");
        }
      }
      throw error;
    }

    const metadata = await pdfDocument.getMetadata();

    return {
      numPages: pdfDocument.numPages,
      data,
      metadata,
      _pdfDocument: pdfDocument,
    } as PdfJsExtendedDocument;
  }

  async extractPage(doc: PdfDocument, pageNum: number): Promise<PageData> {
    const pdfDocument = (doc as PdfJsExtendedDocument)._pdfDocument;
    const page = await pdfDocument.getPage(pageNum);

    // Get viewport
    const viewport = page.getViewport({ scale: 1.0 });

    // Extract text content
    const textContent = await page.getTextContent();
    const viewportWidth = viewport.width;
    const viewportHeight = viewport.height;
    const viewportTransform = viewport.transform;

    const textItems: TextItem[] = [];
    const garbledTextRegions: BoundingBox[] = [];
    for (const item of textContent.items) {
      // Skip items with zero dimensions
      if (item.height === 0 || item.width === 0) continue;

      // Apply viewport transformation to convert PDF coordinates to screen coordinates
      // This properly handles Y-axis flip (PDF is bottom-up, screen is top-down)
      const cm = multiplyMatrices(viewportTransform, item.transform);

      // Get lower-left corner (text space origin)
      const ll = applyTransformation({ x: 0, y: 0 }, cm);

      // Extract scale factors directly from matrix components (not SVD).
      // For matrix [a, b, c, d, tx, ty]:
      // - Horizontal scale = sqrt(a² + b²)
      // - Vertical scale = sqrt(c² + d²)
      // This correctly preserves axis association unlike SVD which returns
      // singular values sorted by magnitude (causing x/y swap for some fonts).
      const scaleX = Math.sqrt(item.transform[0] ** 2 + item.transform[1] ** 2);
      const scaleY = Math.sqrt(item.transform[2] ** 2 + item.transform[3] ** 2);

      // Get upper-right corner by first converting width/height to text space
      // (dividing by the scale factors), then transforming to viewport space
      const ur = applyTransformation({ x: item.width / scaleX, y: item.height / scaleY }, cm);

      // Calculate final bounding box in viewport space
      const left = Math.min(ll.x, ur.x);
      const right = Math.max(ll.x, ur.x);
      const top = Math.min(ll.y, ur.y);
      const bottom = Math.max(ll.y, ur.y);

      // Skip items that are off-page (negative coordinates or beyond page bounds)
      if (top < 0 || left < 0 || top > viewportHeight || left > viewportWidth) continue;

      const width = right - left;
      const height = bottom - top;

      // Calculate rotation from combined transformation matrix
      let rotation = getRotation(cm);
      // Normalize to 0-360 range
      if (rotation < 0) {
        rotation += 360;
      }

      // Decode buggy font markers from PDF.js (only if marker is present)
      // Format: :->|>_<charCode>_<fontChar>_<|<-:
      let decodedStr = item.str;
      if (decodedStr.includes(BUGGY_FONT_MARKER_CHECK)) {
        // Try tabular figures decoding first (common in government/census PDFs)
        const tabularDecoded = tryDecodeTabularFigures(decodedStr);
        if (tabularDecoded) {
          decodedStr = tabularDecoded;
        } else {
          // Fall back to original approach: use glyph ID as character code
          BUGGY_FONT_MARKER_REGEX.lastIndex = 0; // Reset regex state
          decodedStr = decodedStr.replace(BUGGY_FONT_MARKER_REGEX, (_: string, charCode: string) =>
            String.fromCharCode(parseInt(charCode))
          );
        }
      }

      // Handle pipe-separated characters: " |a|  |r|  |X| " -> "arX"
      // Some PDFs encode text with characters separated by pipes and spaces
      if (decodedStr.includes("|")) {
        PIPE_PATTERN_REGEX.lastIndex = 0; // Reset regex state
        const matches = [...decodedStr.matchAll(PIPE_PATTERN_REGEX)];
        if (matches.length > 0) {
          decodedStr = matches.map((m) => m[1]).join("");
        }
      }

      // Skip garbled text from fonts with corrupted ToUnicode mappings
      // Save the bounding box so OCR can fill in these specific regions
      if (isGarbledFontOutput(decodedStr)) {
        garbledTextRegions.push({ x: left, y: top, width, height });
        continue;
      }

      // Strip any remaining control characters from valid text
      // (e.g., form feed chars that sneak into ligatures like "fi")
      decodedStr = stripControlChars(decodedStr);

      textItems.push({
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
      });
    }

    const images: Image[] = [];

    // Skip annotation extraction - not currently used in processing pipeline
    // Can be re-enabled if needed for link extraction, etc.
    const annotations: Annotation[] = [];

    await page.cleanup();

    return {
      pageNum,
      width: viewport.width,
      height: viewport.height,
      textItems,
      images,
      annotations,
      garbledTextRegions: garbledTextRegions.length > 0 ? garbledTextRegions : undefined,
    };
  }

  async extractAllPages(
    doc: PdfDocument,
    maxPages?: number,
    targetPages?: string
  ): Promise<PageData[]> {
    const numPages = Math.min(doc.numPages, maxPages || doc.numPages);

    const pages: PageData[] = [];

    // Parse target pages if specified
    let pageNumbers: number[];
    if (targetPages) {
      pageNumbers = this.parseTargetPages(targetPages, doc.numPages);
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

  async renderPageImage(_doc: PdfDocument, pageNum: number, dpi: number, password?: string): Promise<Buffer> {
    if (!this.pdfiumRenderer) {
      this.pdfiumRenderer = new PdfiumRenderer();
    }

    // Use file path when available, otherwise pass buffer directly
    const pdfInput = this.currentPdfPath || this.currentPdfData;
    if (!pdfInput) {
      throw new Error("No PDF path or data available for rendering");
    }

    return await this.pdfiumRenderer.renderPageToBuffer(pdfInput, pageNum, dpi, password);
  }

  async close(doc: PdfDocument): Promise<void> {
    const pdfDocument = (doc as PdfJsExtendedDocument)._pdfDocument;
    if (pdfDocument && pdfDocument.destroy) {
      await pdfDocument.destroy();
    }

    // Clean up PDFium renderer (only if it was initialized)
    if (this.pdfiumRenderer) {
      await this.pdfiumRenderer.close();
      this.pdfiumRenderer = null;
    }
    this.currentPdfPath = null;
    this.currentPdfData = null;
  }

  private parseTargetPages(targetPages: string, maxPages: number): number[] {
    const pages: number[] = [];
    const parts = targetPages.split(",");

    for (const part of parts) {
      const trimmed = part.trim();
      if (trimmed.includes("-")) {
        // Range: "1-5"
        const [start, end] = trimmed.split("-").map((n) => parseInt(n.trim()));
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
