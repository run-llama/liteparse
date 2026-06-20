import {
  native,
  type LiteParseNative,
  type LiteParseNativeConfig,
  type NativeParseResult,
  type NativeParsedPage,
  type NativePageInput,
  type NativeTextItem,
  type NativeExtractedImage,
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
  /** Inline embedded PDF images into markdown text as base64 data URI images. */
  inlineImages: boolean;
  /** Render hyperlink annotations as `[text](url)` in markdown output (default: true). */
  extractLinks: boolean;
  preserveVerySmallText: boolean;
  password?: string;
  quiet: boolean;
  numWorkers: number;
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
  textItems: TextItem[];
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
      inlineImages: userConfig.inlineImages,
      extractLinks: userConfig.extractLinks,
      preserveVerySmallText: userConfig.preserveVerySmallText,
      password: userConfig.password,
      quiet: userConfig.quiet,
      numWorkers: userConfig.numWorkers,
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
      inlineImages: resolved.inlineImages ?? false,
      extractLinks: resolved.extractLinks ?? true,
      preserveVerySmallText: resolved.preserveVerySmallText ?? false,
      password: resolved.password ?? undefined,
      quiet: resolved.quiet ?? false,
      numWorkers: resolved.numWorkers ?? 1,
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

function toPage(p: NativeParsedPage): ParsedPage {
  return {
    pageNum: p.pageNum,
    width: p.width,
    height: p.height,
    text: p.text,
    textItems: p.textItems.map(toTextItem),
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
