/**
 * Supported output formats for parsed documents.
 *
 * - `"json"` — Structured JSON with per-page text items, bounding boxes, and metadata.
 * - `"text"` — Plain text with spatial layout preserved.
 */
import type { GridDebugConfig } from "../processing/gridDebugLogger.js";

export type OutputFormat = "json" | "text";

/**
 * Accepted input types for {@link LiteParse.parse} and {@link LiteParse.screenshot}.
 *
 * - `string` — A file path to a document on disk.
 * - `Buffer | Uint8Array` — Raw file bytes (PDF bytes go straight to the parser with zero disk I/O;
 *   non-PDF bytes are written to a temp file for format conversion).
 */
export type LiteParseInput = string | Buffer | Uint8Array;

/**
 * Configuration options for the {@link LiteParse} parser.
 *
 * All fields have sensible defaults. Pass a `Partial<LiteParseConfig>` to the
 * constructor to override only the options you need.
 *
 * @example
 * ```typescript
 * const parser = new LiteParse({
 *   ocrEnabled: true,
 *   ocrLanguage: "fra",
 *   dpi: 300,
 *   outputFormat: "json",
 * });
 * ```
 */
export interface LiteParseConfig {
  /**
   * OCR language code(s). Uses ISO 639-3 codes for Tesseract (e.g., `"eng"`, `"fra"`)
   * or ISO 639-1 for HTTP OCR servers (e.g., `"en"`, `"fr"`).
   *
   * @defaultValue `"en"`
   */
  ocrLanguage: string | string[];

  /**
   * Whether to run OCR on pages with little or no native text.
   * When enabled, LiteParse selectively OCRs only images and text-sparse regions.
   *
   * @defaultValue `true`
   */
  ocrEnabled: boolean;

  /**
   * URL of an HTTP OCR server implementing the LiteParse OCR API.
   * If not provided, the built-in Tesseract.js engine is used.
   *
   * @see {@link https://github.com/run-llama/liteparse/blob/main/OCR_API_SPEC.md | OCR API Specification}
   */
  ocrServerUrl?: string;

  /**
   * Path to a directory containing Tesseract `.traineddata` files.
   * Used as both the language data source and cache directory for Tesseract.js.
   *
   * If not set, falls back to the `TESSDATA_PREFIX` environment variable.
   * If neither is set, Tesseract.js downloads data from cdn.jsdelivr.net.
   *
   * @example `/opt/tessdata`
   */
  tessdataPath?: string;

  /**
   * Number of pages to OCR in parallel. Higher values use more memory but
   * process faster on multi-core machines.
   *
   * @defaultValue CPU cores - 1 (minimum 1)
   */
  numWorkers: number;

  /**
   * Maximum number of pages to parse from the document.
   *
   * @defaultValue `1000`
   */
  maxPages: number;

  /**
   * Specific pages to parse, as a comma-separated string of page numbers and ranges.
   *
   * @example `"1-5,10,15-20"`
   */
  targetPages?: string;

  /**
   * DPI (dots per inch) for rendering pages to images. Higher values improve
   * OCR accuracy but increase processing time and memory usage.
   *
   * @defaultValue `150`
   */
  dpi: number;

  /**
   * Output format for parsed results.
   *
   * @defaultValue `"json"`
   */
  outputFormat: OutputFormat;

  /**
   * Calculate precise bounding boxes for each text line. Disable for faster
   * parsing when bounding boxes aren't needed.
   *
   * @deprecated Controls the deprecated `boundingBoxes` output. Will be removed in v2.0.
   * Text item coordinates (`x`, `y`, `width`, `height`) are always present regardless.
   *
   * @defaultValue `true`
   */
  preciseBoundingBox: boolean;

  /**
   * Preserve very small text that would normally be filtered out.
   *
   * @defaultValue `false`
   */
  preserveVerySmallText: boolean;

  /**
   * Maintain consistent text alignment across page boundaries.
   *
   * @defaultValue `false`
   */
  preserveLayoutAlignmentAcrossPages: boolean;

  /**
   * Password for opening encrypted/protected documents.
   * Used for password-protected PDFs and office documents.
   *
   * @defaultValue `undefined`
   */
  password?: string;

  /**
   * Debug configuration for grid projection. When enabled, logs detailed
   * information about how text elements are snapped, anchored, and projected.
   * Can also generate visual PNG overlays of the projection.
   *
   * @example
   * ```typescript
   * const parser = new LiteParse({
   *   debug: {
   *     enabled: true,
   *     textFilter: ["Total", "Revenue"],
   *     pageFilter: 2,
   *     visualize: true,
   *     visualizePath: "./debug-output",
   *   }
   * });
   * ```
   */
  debug?: GridDebugConfig;
}

/**
 * An individual text element extracted from a page, with position, size, and font metadata.
 *
 * Coordinates use the PDF coordinate system where the origin is at the top-left
 * of the page, x increases to the right, and y increases downward.
 */
export interface TextItem {
  /** The text content of this item. */
  str: string;
  /** X coordinate of the top-left corner, in PDF points. */
  x: number;
  /** Y coordinate of the top-left corner, in PDF points. */
  y: number;
  /** Width of the text item in PDF points. */
  width: number;
  /** Height of the text item in PDF points. */
  height: number;
  /** Alias for {@link TextItem.width | width}. */
  w: number;
  /** Alias for {@link TextItem.height | height}. */
  h: number;
  /** Font name (e.g., `"Helvetica"`, `"Times-Roman"`, `"OCR"` for OCR-detected text). */
  fontName?: string;
  /** Font size in PDF points. */
  fontSize?: number;
  /** Rotation angle in degrees. One of `0`, `90`, `180`, or `270`. */
  r?: number;
  /** X coordinate after rotation transformation. */
  rx?: number;
  /** Y coordinate after rotation transformation. */
  ry?: number;
  /** Markup annotations (highlights, underlines, etc.) applied to this text. */
  markup?: MarkupData;
  /** @internal Whether this item represents a vertical gap. */
  vgap?: boolean;
  /** @internal Whether this is a placeholder item used during layout. */
  isPlaceholder?: boolean;
  /** Confidence score from 0.0 to 1.0. Native PDF text defaults to 1.0, OCR text reflects engine confidence. */
  confidence?: number;
}

/**
 * Markup annotation data associated with a text item.
 */
export interface MarkupData {
  /** Highlight color (e.g., `"yellow"`, `"#FFFF00"`), or `undefined` if not highlighted. */
  highlight?: string;
  /** Whether the text is underlined. */
  underline?: boolean;
  /** Whether the text has a squiggly underline. */
  squiggly?: boolean;
  /** Whether the text is struck out. */
  strikeout?: boolean;
}

export interface ProjectionTextBox {
  str: string;
  x: number;
  y: number;
  w: number;
  h: number;
  rx?: number;
  ry?: number;
  r?: number;
  strLength: number;
  markup?: MarkupData;
  pageBbox?: Coordinates;
  vgap?: boolean;
  isPlaceholder?: boolean;
  fromOCR?: boolean;

  // Projection metadata
  snap?: "left" | "right" | "center";
  leftAnchor?: string;
  rightAnchor?: string;
  centerAnchor?: string;
  isDup?: boolean;
  rendered?: boolean;
  isMarginLineNumber?: boolean;
  shouldSpace?: number;
  forceUnsnapped?: boolean;
  rotated?: boolean;
  d?: number; // Delta for rotation handling
}

/**
 * A rectangle defined by position and dimensions.
 * @internal
 */
export interface Coordinates {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Raw OCR detection result before conversion to {@link TextItem}.
 * @internal
 */
export interface OcrData {
  x: number;
  y: number;
  w: number;
  h: number;
  /** Confidence score from 0.0 to 1.0. */
  confidence: number;
  /** Recognized text. */
  text: string;
}

/**
 * An axis-aligned bounding box defined by its top-left and bottom-right corners.
 *
 * All coordinates are in PDF points.
 *
 * @deprecated Use {@link TextItem} coordinates (`x`, `y`, `width`, `height`) instead. Will be removed in v2.0.
 */
export interface BoundingBox {
  /** X coordinate of the top-left corner. */
  x1: number;
  /** Y coordinate of the top-left corner. */
  y1: number;
  /** X coordinate of the bottom-right corner. */
  x2: number;
  /** Y coordinate of the bottom-right corner. */
  y2: number;
}

/**
 * Parsed data for a single page of a document.
 */
export interface ParsedPage {
  /** 1-indexed page number. */
  pageNum: number;
  /** Page width in PDF points. */
  width: number;
  /** Page height in PDF points. */
  height: number;
  /** Full text content of the page with spatial layout preserved. */
  text: string;
  /** Individual text elements extracted from the page. */
  textItems: TextItem[];
  /**
   * @deprecated Use {@link TextItem} coordinates instead. Will be removed in v2.0.
   * Present when {@link LiteParseConfig.preciseBoundingBox} is enabled.
   */
  boundingBoxes?: BoundingBox[];
}

/**
 * A text element from the JSON output with position, size, and font metadata.
 */
export interface JsonTextItem {
  /** The text content of this item. */
  text: string;
  /** X coordinate of the top-left corner, in PDF points. */
  x: number;
  /** Y coordinate of the top-left corner, in PDF points. */
  y: number;
  /** Width of the text item in PDF points. */
  width: number;
  /** Height of the text item in PDF points. */
  height: number;
  /** Font name. */
  fontName?: string;
  /** Font size in PDF points. */
  fontSize?: number;
  /** The OCR confidence (null if OCR wasn't used) */
  confidence?: number;
}

/**
 * Options for {@link searchItems}.
 */
export interface SearchItemsOptions {
  /** Find text items containing this phrase. Matches can span multiple adjacent items. */
  phrase: string;
  /**
   * Whether the search should be case-sensitive.
   *
   * @defaultValue `false`
   */
  caseSensitive?: boolean;
}

/**
 * Structured JSON representation of parsed document data.
 * Returned when {@link LiteParseConfig.outputFormat} is `"json"`.
 */
export interface ParseResultJson {
  /** Array of page data. */
  pages: Array<{
    /** 1-indexed page number. */
    page: number;
    /** Page width in PDF points. */
    width: number;
    /** Page height in PDF points. */
    height: number;
    /** Full text content of the page. */
    text: string;
    /** Individual text elements with position and font metadata. */
    textItems: JsonTextItem[];
    /**
     * @deprecated Use `textItems` coordinates instead. Will be removed in v2.0.
     */
    boundingBoxes: BoundingBox[];
  }>;
}

/**
 * The result of parsing a document with {@link LiteParse.parse}.
 */
export interface ParseResult {
  /** Per-page parsed data. */
  pages: ParsedPage[];
  /** Full document text, concatenated from all pages. */
  text: string;
  /** Structured JSON data. Present when {@link LiteParseConfig.outputFormat} is `"json"`. */
  json?: ParseResultJson;
}

/**
 * The result of generating a screenshot with {@link LiteParse.screenshot}.
 */
export interface ScreenshotResult {
  /** 1-indexed page number. */
  pageNum: number;
  /** Image width in pixels. */
  width: number;
  /** Image height in pixels. */
  height: number;
  /** Raw image data as a Node.js Buffer (PNG or JPG). */
  imageBuffer: Buffer;
  /** File path if the screenshot was saved to disk. */
  imagePath?: string;
}
