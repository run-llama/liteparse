import {
  native,
  type LiteParseNative,
  type LiteParseNativeConfig,
  type NativeParseResult,
  type NativeParsedPage,
  type NativePageInput,
  type NativeTextItem,
  type NativeExtractedImage,
  type NativePageComplexityStats,
} from "./native.js";

// ---------------------------------------------------------------------------
// Public types — match the existing TypeScript API
// ---------------------------------------------------------------------------

export type LiteParseInput = string | Buffer | Uint8Array;
export type OutputFormat = "json" | "text" | "markdown";
export type ImageMode = "off" | "placeholder" | "embed";

export interface LiteParseConfig {
  ocrLanguage: string;
  ocrEnabled: boolean;
  ocrServerUrl?: string;
  /** Extra HTTP headers sent with every request to `ocrServerUrl`. */
  ocrServerHeaders?: Record<string, string>;
  tessdataPath?: string;
  maxPages: number;
  targetPages?: string;
  dpi: number;
  outputFormat: OutputFormat;
  /** How to surface raster images in markdown output (default: "placeholder"). */
  imageMode: ImageMode;
  /** Render hyperlink annotations as `[text](url)` in markdown output (default: true). */
  extractLinks: boolean;
  /** Extract all PDF annotations into each parsed page (default: false). */
  extractAnnotations: boolean;
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
   */
  emitWordBoxes: boolean;
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

export interface ParsedPage {
  pageNum: number;
  width: number;
  height: number;
  text: string;
  markdown: string;
  textItems: TextItem[];
  /**
   * Per-page complexity signals (the same {@link LiteParse.isComplex} returns).
   * Present only when parsing with `includeComplexity: true`; `undefined`
   * otherwise.
   */
  complexity?: PageComplexityStats;
  /** Present only when `extractAnnotations` is enabled. */
  annotations?: DocumentAnnotation[];
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

export interface ExtractedImage {
  /** Reference id used in the markdown output (e.g. `![](image_p1_0.png)` → `"p1_0"`). */
  id: string;
  page: number;
  format: string;
  bytes: Buffer;
}

export interface ParseResult {
  pages: ParsedPage[];
  text: string;
  /** Populated only when configured with `imageMode: "embed"`. */
  images: ExtractedImage[];
}

export interface ScreenshotResult {
  pageNum: number;
  width: number;
  height: number;
  imageBuffer: Buffer;
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
  imageBlockCount: number;
  /** Summed image-bbox area over page area, clamped to 1. */
  imageCoverage: number;
  /** Largest single *counted* image's area over page area, clamped to 1. */
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

  constructor(userConfig: Partial<LiteParseConfig> = {}) {
    const nativeConfig: LiteParseNativeConfig = {
      ocrLanguage: userConfig.ocrLanguage,
      ocrEnabled: userConfig.ocrEnabled,
      ocrServerUrl: userConfig.ocrServerUrl,
      ocrServerHeaders: userConfig.ocrServerHeaders,
      tessdataPath: userConfig.tessdataPath,
      maxPages: userConfig.maxPages,
      targetPages: userConfig.targetPages,
      dpi: userConfig.dpi,
      outputFormat: userConfig.outputFormat,
      imageMode: userConfig.imageMode,
      extractLinks: userConfig.extractLinks,
      extractAnnotations: userConfig.extractAnnotations,
      preserveVerySmallText: userConfig.preserveVerySmallText,
      password: userConfig.password,
      quiet: userConfig.quiet,
      numWorkers: userConfig.numWorkers,
      ocrFailureFatal: userConfig.ocrFailureFatal,
      ocrHedgeDelaysMs: userConfig.ocrHedgeDelaysMs,
      emitWordBoxes: userConfig.emitWordBoxes,
      cropBox: userConfig.cropBox,
      skipDiagonalText: userConfig.skipDiagonalText,
      includeComplexity: userConfig.includeComplexity,
    };

    this._native = new native.LiteParse(nativeConfig);

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
      dpi: resolved.dpi ?? 150,
      outputFormat: (resolved.outputFormat as OutputFormat) ?? "json",
      imageMode: (resolved.imageMode as ImageMode) ?? "placeholder",
      extractLinks: resolved.extractLinks ?? true,
      extractAnnotations: resolved.extractAnnotations ?? false,
      preserveVerySmallText: resolved.preserveVerySmallText ?? false,
      password: resolved.password ?? undefined,
      quiet: resolved.quiet ?? false,
      numWorkers: resolved.numWorkers ?? 1,
      ocrFailureFatal: resolved.ocrFailureFatal ?? true,
      ocrHedgeDelaysMs: resolved.ocrHedgeDelaysMs ?? [],
      emitWordBoxes: resolved.emitWordBoxes ?? false,
      cropBox: resolved.cropBox ?? undefined,
      skipDiagonalText: resolved.skipDiagonalText ?? false,
      includeComplexity: resolved.includeComplexity ?? false,
    };
  }

  async parse(input: LiteParseInput): Promise<ParseResult> {
    // Convert Uint8Array to Buffer for the native side
    const nativeInput =
      typeof input === "string" ? input : Buffer.from(input);
    const result: NativeParseResult = await this._native.parse(nativeInput);
    return {
      pages: result.pages.map(toPage),
      text: result.text,
      images: (result.images ?? []).map(toImage),
    };
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
    return {
      pages: result.pages.map(toPage),
      text: result.text,
      images: (result.images ?? []).map(toImage),
    };
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

function toPage(p: NativeParsedPage): ParsedPage {
  return {
    pageNum: p.pageNum,
    width: p.width,
    height: p.height,
    text: p.text,
    markdown: p.markdown,
    textItems: p.textItems.map(toTextItem),
    complexity: p.complexity ? toComplexity(p.complexity) : undefined,
    annotations: p.annotations,
  };
}

function toImage(img: NativeExtractedImage): ExtractedImage {
  return {
    id: img.id,
    page: img.page,
    format: img.format,
    bytes: img.bytes,
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
