import {
  native,
  type LiteParseNative,
  type LiteParseNativeConfig,
  type NativeParseResult,
  type NativeParsedPage,
  type NativePageInput,
  type NativeTextItem,
  type NativeExtractedImage,
  type NativeStructureTreeElement,
  type NativePageComplexityStats,
  type NativeScreenshotResult,
} from "./native.js";
import { WorkerPool, ParseTimeoutError } from "./pool.js";

export { ParseTimeoutError };

// ---------------------------------------------------------------------------
// Public types — match the existing TypeScript API
// ---------------------------------------------------------------------------

export type LiteParseInput = string | Buffer | Uint8Array;
export type OutputFormat = "json" | "text" | "markdown";
export type ImageMode = "off" | "placeholder" | "embed";

/** Options for pool mode: parsing in persistent, killable worker processes. */
export interface PoolOptions {
  /**
   * Route `parse()` through a pool of this many persistent worker processes.
   * Call `close()` when done (an idle pool never keeps the event loop alive, 
   * but explicit shutdown frees workers immediately).
   */
  poolSize?: number;
  /**
   * Hard per-parse deadline in milliseconds. Requires `poolSize`.
   * The pool enforces the deadline by SIGKILLing the worker. On expiry
   * the parse rejects with {@link ParseTimeoutError} (naming the document)
   * and a fresh worker replaces the killed one.
   */
  parseTimeoutMs?: number;
}

export interface LiteParseConfig {
  ocrLanguage: string;
  ocrEnabled: boolean;
  ocrServerUrl?: string;
  /** Extra HTTP headers sent with every request to `ocrServerUrl`. */
  ocrServerHeaders?: Record<string, string>;
  tessdataPath?: string;
  maxPages: number;
  targetPages?: string;
  /** Render parsed pages to PNG and return them in `ParseResult.screenshots`. */
  extractScreenshots: boolean;
  /** Continue after page-level extraction failures and collect `pageErrors`. */
  continueOnPageError: boolean;
  dpi: number;
  outputFormat: OutputFormat;
  /** How to surface raster images in markdown output (default: "placeholder"). */
  imageMode: ImageMode;
  /** Extract embedded image bytes and metadata (default: false). */
  extractImages: boolean;
  /** Directory where extracted embedded image files are written. Requires `extractImages`. */
  imageOutputDir?: string;
  /** Render hyperlink annotations as `[text](url)` in markdown output (default: true). */
  extractLinks: boolean;
  /** Keep running headers/footers in markdown output instead of stripping repeated page-band lines and page chrome (default: false). */
  keepHeadersFooters: boolean;
  /** Extract all PDF annotations into each parsed page (default: false). */
  extractAnnotations: boolean;
  /** Extract AcroForm widget fields and values (default: false). */
  extractFormFields: boolean;
  /** Extract the tagged-PDF logical structure tree (default: false). */
  extractStructureTree: boolean;
  /**
   * Emit each page's classified layout blocks with bounding boxes
   * (default: false). This is the same decomposition the Markdown renderer
   * consumes, exposed as data; enabling it never changes the rendered Markdown.
   */
  extractBlocks: boolean;
  /** Extract raw XFA packets (name + XML content) into `ParseResult.xfaPackets` (default: false). */
  extractXfaPackets: boolean;
  /**
   * Collect document provenance metadata into `result.docMeta`. Default
   * false: Absent for inputs converted from a non-PDF format.
   */
  extractDocumentMetadata: boolean;
  /** Emit each page's `contentBounds` (union bbox of top-level content objects) (default: false). */
  extractContentBounds: boolean;
  /** Detect solid rectangles/lines in rendered page screenshots (default: false). */
  detectScreenshotRects: boolean;
  /** Draw AcroForm field appearances into rendered rasters (screenshots and OCR inputs; runs document open/JS actions; default: false). */
  renderFormFields: boolean;
  preserveVerySmallText: boolean;
  password?: string;
  quiet: boolean;
  numWorkers: number;
  /**
   * Whether a systemic OCR failure (every OCR task failed and at least one was
   * a text-sparse page) aborts the whole parse (default: true). Set false to
   * keep already-recovered native text and return partial results instead of
   * rejecting — for callers that prefer a degraded document over a hard failure.
   */
  ocrFailureFatal: boolean;
  /**
   * OCR request-hedging schedule (ms). Empty (default) = no hedging. Multiple
   * delays (e.g. `[0, 5000, 10000, 15000, 20000]`) fire duplicate requests per
   * OCR attempt and take the first success — lower tail latency on a slow/stuck
   * OCR pod, at the cost of extra OCR-server load. HTTP OCR engine only.
   */
  ocrHedgeDelaysMs: number[];
  /**
   * Emit per-word sub-boxes on each text item ({@link TextItem.words}).
   * Default false. Word boxes roughly double the text-item payload (size + napi
   * marshalling), so enable only when doing word-level bbox attribution.
   * With `extractBlocks` on, word geometry is always computed internally as a
   * table-detection input, but `words` is only returned when this is set.
   */
  emitWordBoxes: boolean;
  /** Include rich PDF text metadata on returned text items. Default false. */
  extractTextMetadata?: boolean;
  /**
   * Restrict output to a page sub-region. Each field is the fraction of the
   * page cropped away from that side (top-left origin), so `{ left: 0.5 }`
   * discards the left half. A text item survives only when it lies entirely
   * inside the remaining rectangle. Undefined (default) keeps the whole page.
   * Applied after OCR merge, so OCR text outside the region is dropped too.
   */
  cropBox?: CropBox;
  /**
   * Drop diagonal text — items whose rotation is more than 2° off the nearest
   * right angle (0/90/180/270). Default false. Use to exclude rotated
   * watermarks/stamps from the output.
   */
  skipDiagonalText: boolean;
  /**
   * Compute per-page complexity signals during {@link LiteParse.parse} and
   * attach them to each page as {@link ParsedPage.complexity} (the same signals
   * {@link LiteParse.isComplex} returns). Default false; enabling it runs an
   * extra vector-text detection pass.
   */
  includeComplexity: boolean;
  /** Expose page-scoped vector shapes and merged H/V line segments. Default false. */
  extractVectorGraphics: boolean;
}

/**
 * A page sub-region expressed as the fraction cropped from each side
 * (top-left origin, each value in `[0, 1]`).
 */
export interface CropBox {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

/**
 * One word's bounding box within a {@link TextItem}, in the same viewport space
 * (top-left origin, 72 DPI). `text` excludes inter-word spaces.
 */
export interface WordBox {
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface TextItem {
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  fontName?: string;
  fontSize?: number;
  /** Font size after applying the text matrix's vertical scale. */
  fontHeight?: number;
  fontAscent?: number;
  fontDescent?: number;
  fontWeight?: number;
  /** Sum of source glyph widths in points. */
  textWidth?: number;
  fontIsBuggy?: boolean;
  /** Marked-content ID from the PDF structure tree. */
  mcid?: number;
  /** Fill color as an eight-character ARGB hex string. */
  fillColor?: string;
  /** Stroke color as an eight-character ARGB hex string. */
  strokeColor?: string;
  /** Raw PDF content-stream character codes for the source glyphs. */
  charCodes?: number[];
  /** True when the trailing source space was synthesized by PDFium. */
  trailingSpaceGenerated?: boolean;
  /** OCR confidence score (0.0-1.0). Undefined for native PDF text. */
  confidence?: number;
  /** Rotation in degrees (viewport space). Defaults to 0 when omitted. */
  rotation?: number;
  /**
   * Per-word sub-boxes within this item. A text item groups several words
   * together (breaking only at line/column boundaries), so this carries the
   * finer word-level geometry for bbox attribution. Empty/undefined for items
   * with no word split (e.g. OCR-sourced or single-token items).
   */
  words?: WordBox[];
}

/**
 * A vector-graphic primitive supplied to {@link LiteParse.parsePages}. `kind`
 * selects the variant: `"stroke"` (uses `x1/y1/x2/y2`) or `"rect"` (uses
 * `x/y/width/height`, top-left origin). Coordinates are viewport space (72 DPI),
 * matching the text items. `hasFill`/`hasStroke` carry the paint intent even
 * when the color is unknown, so ruled-table edge detection still treats a
 * colorless stroked rect as stroked.
 */
export interface Graphic {
  kind: "stroke" | "rect";
  x1?: number;
  y1?: number;
  x2?: number;
  y2?: number;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  hasFill?: boolean;
  hasStroke?: boolean;
  fillColor?: string;
  strokeColor?: string;
  lineWidth?: number;
}

/**
 * A page of pre-extracted text supplied to {@link LiteParse.parsePages}.
 * Coordinates are viewport space (top-left origin, 72 DPI). `graphics` is
 * optional; when supplied it enables ruled-table and horizontal-rule detection.
 */
export interface PageInput {
  pageNumber: number;
  pageWidth: number;
  pageHeight: number;
  textItems: TextItem[];
  graphics?: Graphic[];
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ParsedPage {
  pageNum: number;
  width: number;
  height: number;
  /**
   * Union bbox of the page's top-level content objects in viewport coords
   * (visible content extent). Absent for empty pages.
   */
  /** Present only when `extractContentBounds` is enabled. */
  contentBounds?: Rect;
  text: string;
  markdown: string;
  textItems: TextItem[];
  /**
   * Per-page complexity signals (the same {@link LiteParse.isComplex} returns).
   * Present only when parsing with `includeComplexity: true`; `undefined`
   * otherwise.
   */
  complexity?: PageComplexityStats;
  /** Present only when parsing with `extractVectorGraphics: true`. */
  vectorGraphics?: VectorGraphics;
  /** Present only when `extractAnnotations` is enabled. */
  annotations?: DocumentAnnotation[];
  /** Present only when `extractFormFields` is enabled. */
  formFields?: FormField[];
  /** Present only when `extractStructureTree` is enabled. */
  structureTree?: StructureTree;
  /**
   * Classified layout blocks in reading order — the same blocks, in the same
   * order, the page's Markdown is built from. Present only when
   * `extractBlocks` is enabled.
   */
  blocks?: LayoutBlock[];
}

/** One table cell: its text and the region of the page it was read from. */
export interface LayoutCell {
  text: string;
  /**
   * Absent for cells with no ink behind them — padding inserted to square off
   * a ragged grid, or halves of a merged run split at an estimated position.
   */
  bbox?: Rect;
  /**
   * Indices into the page's returned `textItems`, in reading order, never
   * repeating within one cell; empty for padding cells.
   */
  textItemIndices: number[];
}

/** A classified block of page content, discriminated by `kind`. */
export interface LayoutBlock {
  kind:
    | "heading"
    | "paragraph"
    | "list_item"
    | "code"
    | "table"
    | "grid_fallback"
    | "rule"
    | "figure";
  /**
   * Indices into the page's returned `textItems`, sorted and deduped; empty
   * for text-less blocks. For a `table` block, the union of its cells'
   * indices.
   */
  textItemIndices: number[];
  /** Rendered text for `heading`, `paragraph`, and `list_item`. */
  text?: string;
  /** Heading level (1-6), or list nesting depth for `list_item`. */
  level?: number;
  bold?: boolean;
  italic?: boolean;
  /** `list_item` only. `marker` is the marker as it appeared on the page. */
  ordered?: boolean;
  marker?: string;
  /** Verbatim source lines for `code` and `grid_fallback`. */
  lines?: string[];
  /** Best-effort language hint for `code`. */
  lang?: string;
  /** `table` only. */
  header?: LayoutCell[];
  rows?: LayoutCell[][];
  /** `figure` only, matching the `img_{id}.{format}` Markdown target. */
  id?: string;
  format?: string;
  /**
   * Region this block occupies, in the same top-left 72-DPI viewport space as
   * `textItems`. The union of every source line that fed the block.
   */
  bbox?: Rect;
}

export type StructureAttributeValue = boolean | number | string;

export interface StructureTree {
  roots: StructureTreeElement[];
}

export interface StructureTreeElement {
  type: string;
  id?: string;
  actualText?: string;
  altText?: string;
  title?: string;
  attributes: Record<string, StructureAttributeValue>;
  markedContentIds: number[];
  children: StructureTreeElement[];
  annotations: DocumentAnnotation[];
}

export interface VectorGraphics {
  shapes: VectorShape[];
  lines: VectorLine[];
}

export interface VectorShape {
  bbox: { x: number; y: number; width: number; height: number };
  stroke: boolean;
  strokeColor?: string;
  fill: boolean;
  fillColor?: string;
  hasCurve: boolean;
}

export interface VectorLine {
  x1: number; y1: number; x2: number; y2: number;
  stroke: boolean;
  strokeWidth?: number;
  strokeColor?: string;
  fill: boolean;
  fillColor?: string;
}

export interface AnnotationRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface DocumentAnnotation {
  subtype: string;
  contents?: string;
  created?: string;
  modified?: string;
  title?: string;
  rect?: AnnotationRect;
  quadpointRects: AnnotationRect[];
  uri?: string;
}

export interface FormField {
  id: string;
  type: string;
  page: number;
  annotationIndex: number;
  widgetIndex: number;
  objectNumber?: number;
  name?: string;
  alternateName?: string;
  value?: string;
  exportValue?: string;
  fieldFlags: number;
  controlCount?: number;
  controlIndex?: number;
  checked?: boolean;
  rect?: AnnotationRect;
  options: string[];
  selectedOptions: string[];
}

export interface ExtractedImage {
  /** Reference id used in the markdown output (e.g. `![](img_p1_1.png)` → `"p1_1"`). */
  id: string;
  /** File name used when `imageOutputDir` is configured. */
  name: string;
  /** Written file path, absent for in-memory-only extraction. */
  path?: string;
  page: number;
  /** Placement on the page in viewport coordinates (top-left origin, 72 DPI). */
  bbox: { x: number; y: number; width: number; height: number };
  /** Intrinsic pixel dimensions of the image resource. */
  width: number;
  height: number;
  /** Clockwise page-object rotation in degrees. */
  rotation: number;
  format: string;
  /** First occurrence with identical encoded source data, when duplicated. */
  duplicateOf?: string;
  bytes: Buffer;
}

export interface ParseResult {
  /** Total source-document pages before `targetPages` or `maxPages` filtering. */
  totalPages: number;
  pages: ParsedPage[];
  /** Page-level PDFium extraction failures when tolerance is enabled. */
  pageErrors: Array<{ pageNum: number; message: string }>;
  text: string;
  /** Populated only when `extractImages` is true. */
  images: ExtractedImage[];
  /** PNG screenshots of parsed pages when `extractScreenshots` is enabled. */
  screenshots: ScreenshotResult[];
  /** Embedded image objects that PDFium could not render or encode. */
  imageErrorCount: number;
  /** PDFium form type, present only when `extractFormFields` is enabled. */
  formType?: number;
  /** The document's `/Info` `Creator` entry, when present. */
  creator?: string;
  /** The document's `/Info` `Producer` entry, when present. */
  producer?: string;
  /**
   * Document-level provenance metadata from PDFium and the source PDF.
   * Present only when `extractDocumentMetadata` is enabled and the input was
   * a real PDF (not converted from DOCX/XLSX/an image).
   */
  docMeta?: DocumentMetadata;
  /** Raw XFA packets; present only when `extractXfaPackets` is enabled. */
  xfaPackets?: XfaPacket[];
}

export interface ParseBatchOptions {
  /** Pages materialized in one batch. Default: 25. */
  batchSize?: number;
}

export interface ParseBatch {
  /** First source page in this batch (1-indexed). */
  startPage: number;
  /** Last source page in this batch (1-indexed, inclusive). */
  endPage: number;
  /** Total source-document pages, before the parser's `maxPages` cap. */
  totalPages: number;
  result: ParseResult;
}

/** Provenance and tamper-analysis facts extracted from the source PDF. */
export interface DocumentMetadata {
  creationDate?: string;
  modDate?: string;
  /** Encoded PDF version (`14` means PDF 1.4). */
  fileVersion?: number;
  isEncrypted?: boolean;
  securityHandlerRevision?: number;
  permissions?: number;
  eofSectionCount?: number;
  startxrefCount?: number;
  trailerIdPairDiffers?: boolean;
  rawFileSize?: number;
  /**
   * The document catalog's `/Metadata` XMP packet, capped at 64 KiB. Absent
   * when the document has none, when it is too large to resolve cheaply, or
   * in WASM builds.
   */
  xmp?: string;
  /** True when the catalog's XMP stream exceeded the 64 KiB cap. */
  xmpTruncated?: boolean;
  signatureCount?: number;
  /** False when bytes were appended after a readable signature byte range. */
  signatureByteRangeReachesEof?: boolean;
}

/** One raw packet from an XFA form document's `/XFA` array. */
export interface XfaPacket {
  index: number;
  name?: string;
  contentLength: number;
  /** Packet content (usually XML), lossily decoded as UTF-8. */
  content?: string;
}

export interface ScreenshotResult {
  pageNum: number;
  width: number;
  height: number;
  imageBuffer: Buffer;
  /** True when every pixel has the same color (blank page after render). */
  isSolidFill: boolean;
  /** Solid rectangles/lines detected in the raster (viewport coords). Populated only with `detectScreenshotRects`. */
  rects: ScreenshotRect[];
}

/** One solid rectangle (or line) detected in a rendered page bitmap. */
export interface ScreenshotRect {
  x: number;
  y: number;
  width: number;
  height: number;
  /** Fill color as ARGB hex string (e.g. "ff1a2b3c"). */
  color: string;
  /** True when the region is a solid line rather than a filled area. */
  isLine: boolean;
}

/**
 * Per-page complexity signals from {@link LiteParse.isComplex}, used to decide
 * whether a document needs OCR or other advanced parsing.
 */
export interface PageComplexityStats {
  pageNumber: number;
  textLength: number;
  /** Fraction of the page area covered by native text (0–1). */
  textCoverage: number;
  hasSubstantialImages: boolean;
  /**
   * Number of counted raster images — inline figures only; full-page
   * backgrounds are excluded (see {@link fullPageImage}).
   */
  imageBlockCount: number;
  /**
   * Summed image-bbox area over page area, clamped to 1. Counts inline figures
   * only: a full-page scan raster contributes 0 here — check
   * {@link fullPageImage} for that.
   */
  imageCoverage: number;
  /**
   * Largest single *counted* image's area over page area, clamped to 1. Same
   * exclusion as {@link imageCoverage}: a full-page raster contributes 0.
   */
  largestImageCoverage: number;
  /**
   * A single raster covers ≥90% of the page. Full-page backgrounds are excluded
   * from the image coverage fields, so this is the only signal that tells a scan
   * apart from a blank page — both otherwise report no text and no images.
   */
  fullPageImage: boolean;
  /**
   * Filled vector-outline area not covered by native text, in pt². `undefined`
   * when a cheaper signal already decided the page, so this walk was skipped.
   */
  uncoveredVectorArea?: number;
  isGarbled: boolean;
  pageArea: number;
  /** Verdict: whether this page needs more than the cheap text-only path. */
  needsOcr: boolean;
  /**
   * Every reason the page was flagged (e.g. `"scanned"`, `"sparse-text"`,
   * `"garbled"`). Empty exactly when `needsOcr` is false. This is the value to
   * route on; new reasons may be added over time.
   */
  reasons: string[];
  /**
   * Layout-difficulty signals (columns, tables, dense graphics). Orthogonal to
   * `needsOcr`: none of these imply OCR — they signal that the text-only path
   * may mangle reading order or structure. Present in `isComplex()` results
   * and `includeComplexity` parses.
   */
  layout?: LayoutComplexityStats;
}

/**
 * Layout-difficulty signals for one page, computed from the real
 * grid-projection pass.
 */
export interface LayoutComplexityStats {
  /** Side-by-side text columns found by the layout pass (1 = single column). */
  columnCount: number;
  /** Ruled-table grids detected on the page. */
  ruledTableCount: number;
  /** Combined ruled-table area over page area, clamped to 1. */
  ruledTableCoverage: number;
  /**
   * Borderless table runs found by track-aligned text detection (description
   * lists excluded). Ruled tables can appear here too — don't sum with
   * `ruledTableCount`; the two discriminate ruled from borderless.
   */
  textTableRunCount: number;
  /** Figure regions clustered from vector graphics. */
  figureCount: number;
  /** Combined figure area over page area, clamped to 1. */
  figureCoverage: number;
  /** Verdict: whether any layout reason fired. */
  isComplex: boolean;
  /**
   * Every layout reason (e.g. `"multi-column"`, `"table-likely"`,
   * `"dense-graphics"`). Empty exactly when `isComplex` is false; new reasons
   * may be added over time.
   */
  reasons: string[];
}

// ---------------------------------------------------------------------------
// LiteParse class
// ---------------------------------------------------------------------------

export class LiteParse {
  private _native: LiteParseNative;
  private _config: LiteParseConfig;
  private _pool: WorkerPool | null = null;

  constructor(userConfig: Partial<LiteParseConfig> & PoolOptions = {}) {
    const nativeConfig: LiteParseNativeConfig = {
      ocrLanguage: userConfig.ocrLanguage,
      ocrEnabled: userConfig.ocrEnabled,
      ocrServerUrl: userConfig.ocrServerUrl,
      ocrServerHeaders: userConfig.ocrServerHeaders,
      tessdataPath: userConfig.tessdataPath,
      maxPages: userConfig.maxPages,
      targetPages: userConfig.targetPages,
      extractScreenshots: userConfig.extractScreenshots,
      continueOnPageError: userConfig.continueOnPageError,
      dpi: userConfig.dpi,
      outputFormat: userConfig.outputFormat,
      imageMode: userConfig.imageMode,
      extractImages: userConfig.extractImages,
      imageOutputDir: userConfig.imageOutputDir,
      extractLinks: userConfig.extractLinks,
      keepHeadersFooters: userConfig.keepHeadersFooters,
      extractAnnotations: userConfig.extractAnnotations,
      extractFormFields: userConfig.extractFormFields,
      extractStructureTree: userConfig.extractStructureTree,
      extractBlocks: userConfig.extractBlocks,
      extractXfaPackets: userConfig.extractXfaPackets,
      extractDocumentMetadata: userConfig.extractDocumentMetadata,
      extractContentBounds: userConfig.extractContentBounds,
      detectScreenshotRects: userConfig.detectScreenshotRects,
      renderFormFields: userConfig.renderFormFields,
      preserveVerySmallText: userConfig.preserveVerySmallText,
      password: userConfig.password,
      quiet: userConfig.quiet,
      numWorkers: userConfig.numWorkers,
      ocrFailureFatal: userConfig.ocrFailureFatal,
      ocrHedgeDelaysMs: userConfig.ocrHedgeDelaysMs,
      emitWordBoxes: userConfig.emitWordBoxes,
      extractTextMetadata: userConfig.extractTextMetadata,
      cropBox: userConfig.cropBox,
      skipDiagonalText: userConfig.skipDiagonalText,
      includeComplexity: userConfig.includeComplexity,
      extractVectorGraphics: userConfig.extractVectorGraphics,
    };

    this._native = new native.LiteParse(nativeConfig);

    if (
      userConfig.parseTimeoutMs !== undefined &&
      userConfig.poolSize === undefined
    ) {
      throw new Error(
        "parseTimeoutMs requires poolSize"
      );
    }
    if (userConfig.poolSize !== undefined) {
      this._pool = new WorkerPool(
        nativeConfig as unknown as Record<string, unknown>,
        userConfig.poolSize,
        userConfig.parseTimeoutMs,
      );
    }

    // Read back the resolved config from the native side
    const resolved = this._native.config;
    this._config = {
      ocrLanguage: resolved.ocrLanguage ?? "eng",
      ocrEnabled: resolved.ocrEnabled ?? true,
      ocrServerUrl: resolved.ocrServerUrl ?? undefined,
      ocrServerHeaders: resolved.ocrServerHeaders ?? undefined,
      tessdataPath: resolved.tessdataPath ?? undefined,
      maxPages: resolved.maxPages ?? 1000,
      targetPages: resolved.targetPages ?? undefined,
      extractScreenshots: resolved.extractScreenshots ?? false,
      continueOnPageError: resolved.continueOnPageError ?? false,
      dpi: resolved.dpi ?? 150,
      outputFormat: (resolved.outputFormat as OutputFormat) ?? "json",
      imageMode: (resolved.imageMode as ImageMode) ?? "placeholder",
      extractImages: resolved.extractImages ?? false,
      imageOutputDir: resolved.imageOutputDir ?? undefined,
      extractLinks: resolved.extractLinks ?? true,
      keepHeadersFooters: resolved.keepHeadersFooters ?? false,
      extractAnnotations: resolved.extractAnnotations ?? false,
      extractFormFields: resolved.extractFormFields ?? false,
      extractStructureTree: resolved.extractStructureTree ?? false,
      extractBlocks: resolved.extractBlocks ?? false,
      extractXfaPackets: resolved.extractXfaPackets ?? false,
      extractDocumentMetadata: resolved.extractDocumentMetadata ?? false,
      extractContentBounds: resolved.extractContentBounds ?? false,
      detectScreenshotRects: resolved.detectScreenshotRects ?? false,
      renderFormFields: resolved.renderFormFields ?? false,
      preserveVerySmallText: resolved.preserveVerySmallText ?? false,
      password: resolved.password ?? undefined,
      quiet: resolved.quiet ?? false,
      numWorkers: resolved.numWorkers ?? 1,
      ocrFailureFatal: resolved.ocrFailureFatal ?? true,
      ocrHedgeDelaysMs: resolved.ocrHedgeDelaysMs ?? [],
      emitWordBoxes: resolved.emitWordBoxes ?? false,
      extractTextMetadata: resolved.extractTextMetadata ?? false,
      cropBox: resolved.cropBox ?? undefined,
      skipDiagonalText: resolved.skipDiagonalText ?? false,
      includeComplexity: resolved.includeComplexity ?? false,
      extractVectorGraphics: resolved.extractVectorGraphics ?? false,
    };
  }

  async parse(input: LiteParseInput): Promise<ParseResult> {
    // Convert Uint8Array to Buffer for the native side
    const nativeInput =
      typeof input === "string" ? input : Buffer.from(input);
    if (this._pool !== null) {
      const source =
        typeof nativeInput === "string"
          ? nativeInput
          : `<${nativeInput.byteLength} bytes>`;
      return this._pool.parse(nativeInput, source);
    }
    const result: NativeParseResult = await this._native.parse(nativeInput);
    return toParseResult(result);
  }

  /**
   * Resolves once every pool worker is initialized. No-op without `poolSize`.
   *
   * Optional: the first parse on each worker waits for its init anyway. Call
   * this before latency-sensitive traffic to avoid paying worker startup on
   * the first request.
   */
  async warmUp(): Promise<void> {
    if (this._pool !== null) await this._pool.warmUp();
  }

  /**
   * Shut down pool workers, if pool mode is enabled. Idempotent.
   *
   * Without `poolSize` this is a no-op. An idle pool never keeps the event
   * loop alive and workers exit when the parent does, so forgetting to call
   * this leaks nothing past process exit.
   */
  close(): void {
    if (this._pool !== null) this._pool.close();
  }

  /**
   * Parse a document in bounded-memory page batches of `batchSize` pages.
   *
   * Each yielded result is independent and becomes collectible once the caller
   * advances the iterator, so a consumer that does not retain batches never
   * holds more than one batch of pages in memory. A non-PDF source is
   * converted once when the iterator starts, not once per batch; its temporary
   * file is released when iteration ends — including an early `break` or
   * `throw`, which run the generator's cleanup.
   *
   * Cross-page passes see only the pages in their own batch, so repeated
   * header/footer removal and image deduplication are batch-local and the
   * output can differ from `parse()`. Prefer `parse()` unless the size of the
   * materialized result is the problem.
   *
   * As with any async generator, work starts on the first `next()` call, so
   * errors (an unreadable file, or a parser configured with `targetPages` —
   * ambiguous with generated batch ranges) surface on the first iteration
   * rather than when `parseBatches()` itself is called.
   */
  async *parseBatches(
    input: LiteParseInput,
    options: ParseBatchOptions = {},
  ): AsyncGenerator<ParseBatch> {
    const nativeInput = typeof input === "string" ? input : Buffer.from(input);
    const session = await this._native.openBatchSession(
      nativeInput,
      options.batchSize,
    );
    try {
      const totalPages = session.totalPages;

      for (;;) {
        const batch = await session.nextBatch();
        if (batch == null) {
          return;
        }
        yield {
          startPage: batch.startPage,
          endPage: batch.endPage,
          totalPages,
          result: toParseResult(batch.result),
        };
      }
    } finally {
      // Frees the session's converted-PDF temp file now instead of at GC —
      // this runs on normal exhaustion and when the consumer abandons the
      // loop early.
      await session.close();
    }
  }

  /**
   * Parse from pre-extracted pages, skipping PDFium text extraction. Runs only
   * grid projection + the configured output formatter, so the caller's own
   * text-extraction / font-recovery owns the text content. Synchronous: no
   * PDFium load and no OCR on this path.
   */
  parsePages(pages: PageInput[]): ParseResult {
    const nativePages: NativePageInput[] = pages.map((p) => ({
      pageNumber: p.pageNumber,
      pageWidth: p.pageWidth,
      pageHeight: p.pageHeight,
      textItems: p.textItems,
      graphics: p.graphics,
    }));
    const result = this._native.parsePages(nativePages);
    return toParseResult(result);
  }

  /**
   * Determine per-page complexity without running a full parse. Returns one
   * entry per page with signals and a `needsOcr` verdict — a cheap pre-OCR
   * check to decide whether a document needs advanced parsing.
   */
  async isComplex(input: LiteParseInput): Promise<PageComplexityStats[]> {
    const nativeInput =
      typeof input === "string" ? input : Buffer.from(input);
    const stats: NativePageComplexityStats[] =
      await this._native.isComplex(nativeInput);
    return stats.map(toComplexity);
  }

  async screenshot(
    input: LiteParseInput,
    pageNumbers?: number[],
  ): Promise<ScreenshotResult[]> {
    const nativeInput =
      typeof input === "string" ? input : Buffer.from(input);
    const results = await this._native.screenshot(
      nativeInput,
      pageNumbers ?? null,
    );
    return results.map((r) => ({
      pageNum: r.pageNum,
      width: r.width,
      height: r.height,
      imageBuffer: r.imageBuffer,
      isSolidFill: r.isSolidFill,
      rects: r.rects,
    }));
  }

  getConfig(): LiteParseConfig {
    return { ...this._config };
  }
}

function toComplexity(s: NativePageComplexityStats): PageComplexityStats {
  return {
    pageNumber: s.pageNumber,
    textLength: s.textLength,
    textCoverage: s.textCoverage,
    hasSubstantialImages: s.hasSubstantialImages,
    imageBlockCount: s.imageBlockCount,
    imageCoverage: s.imageCoverage,
    largestImageCoverage: s.largestImageCoverage,
    fullPageImage: s.fullPageImage,
    uncoveredVectorArea: s.uncoveredVectorArea ?? undefined,
    isGarbled: s.isGarbled,
    pageArea: s.pageArea,
    needsOcr: s.needsOcr,
    reasons: s.reasons,
    layout: s.layout
      ? {
          columnCount: s.layout.columnCount,
          ruledTableCount: s.layout.ruledTableCount,
          ruledTableCoverage: s.layout.ruledTableCoverage,
          textTableRunCount: s.layout.textTableRunCount,
          figureCount: s.layout.figureCount,
          figureCoverage: s.layout.figureCoverage,
          isComplex: s.layout.isComplex,
          reasons: s.layout.reasons,
        }
      : undefined,
  };
}

/** @internal Exported for pool-worker.ts only; not public API. */
export function toParseResult(result: NativeParseResult): ParseResult {
  return {
    totalPages: result.totalPages,
    pages: result.pages.map(toPage),
    pageErrors: result.pageErrors ?? [],
    text: result.text,
    images: (result.images ?? []).map(toImage),
    screenshots: (result.screenshots ?? []).map(toScreenshot),
    imageErrorCount: result.imageErrorCount ?? 0,
    formType: result.formType,
    creator: result.creator,
    producer: result.producer,
    docMeta: result.docMeta,
    xfaPackets: result.xfaPackets,
  };
}

function toPage(p: NativeParsedPage): ParsedPage {
  return {
    pageNum: p.pageNum,
    width: p.width,
    height: p.height,
    contentBounds: p.contentBounds,
    text: p.text,
    markdown: p.markdown,
    textItems: p.textItems.map(toTextItem),
    complexity: p.complexity ? toComplexity(p.complexity) : undefined,
    vectorGraphics: p.vectorGraphics ?? undefined,
    annotations: p.annotations,
    formFields: p.formFields?.map((field) => ({
      id: field.id,
      type: field.fieldType,
      page: field.page,
      annotationIndex: field.annotationIndex,
      widgetIndex: field.widgetIndex,
      objectNumber: field.objectNumber,
      name: field.name,
      alternateName: field.alternateName,
      value: field.value,
      exportValue: field.exportValue,
      fieldFlags: field.fieldFlags,
      controlCount: field.controlCount,
      controlIndex: field.controlIndex,
      checked: field.checked,
      rect: field.rect,
      options: field.options,
      selectedOptions: field.selectedOptions,
    })),
    structureTree: p.structureTree
      ? { roots: p.structureTree.roots.map(toStructureTreeElement) }
      : undefined,
    blocks: p.blocks as LayoutBlock[] | undefined,
  };
}

function toStructureTreeElement(
  element: NativeStructureTreeElement,
): StructureTreeElement {
  const attributes: Record<string, StructureAttributeValue> = {};
  for (const attribute of element.attributes) {
    if (attribute.booleanValue !== undefined) {
      attributes[attribute.name] = attribute.booleanValue;
    } else if (attribute.numberValue !== undefined) {
      attributes[attribute.name] = attribute.numberValue;
    } else if (attribute.stringValue !== undefined) {
      attributes[attribute.name] = attribute.stringValue;
    }
  }
  return {
    type: element.elementType,
    id: element.id,
    actualText: element.actualText,
    altText: element.altText,
    title: element.title,
    attributes,
    markedContentIds: element.markedContentIds,
    children: element.children.map(toStructureTreeElement),
    annotations: element.annotations,
  };
}

function toImage(img: NativeExtractedImage): ExtractedImage {
  return {
    id: img.id,
    name: img.name,
    path: img.path,
    page: img.page,
    bbox: img.bbox,
    width: img.width,
    height: img.height,
    rotation: img.rotation,
    format: img.format,
    duplicateOf: img.duplicateOf,
    bytes: img.bytes,
  };
}

function toScreenshot(result: NativeScreenshotResult): ScreenshotResult {
  return {
    pageNum: result.pageNum,
    width: result.width,
    height: result.height,
    imageBuffer: result.imageBuffer,
    isSolidFill: result.isSolidFill,
    rects: result.rects,
  };
}

function toTextItem(item: NativeTextItem): TextItem {
  return {
    text: item.text,
    x: item.x,
    y: item.y,
    width: item.width,
    height: item.height,
    fontName: item.fontName,
    fontSize: item.fontSize,
    fontHeight: item.fontHeight,
    fontAscent: item.fontAscent,
    fontDescent: item.fontDescent,
    fontWeight: item.fontWeight,
    textWidth: item.textWidth,
    fontIsBuggy: item.fontIsBuggy,
    mcid: item.mcid,
    fillColor: item.fillColor,
    strokeColor: item.strokeColor,
    charCodes: item.charCodes,
    trailingSpaceGenerated: item.trailingSpaceGenerated,
    confidence: item.confidence,
    rotation: item.rotation,
    words: item.words,
  };
}

// ---------------------------------------------------------------------------
// searchItems — standalone utility
// ---------------------------------------------------------------------------

export interface SearchItemsOptions {
  phrase: string;
  caseSensitive?: boolean;
}

export function searchItems(
  items: TextItem[],
  options: SearchItemsOptions,
): TextItem[] {
  const nativeResults = native.searchItems(
    items,
    options.phrase,
    options.caseSensitive ?? false,
  );
  return nativeResults.map(toTextItem);
}

export default LiteParse;
