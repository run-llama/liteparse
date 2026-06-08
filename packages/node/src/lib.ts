import {
  native,
  type LiteParseNative,
  type LiteParseNativeConfig,
  type NativeParseResult,
  type NativeParsedPage,
  type NativeTextItem,
} from "./native.js";

// ---------------------------------------------------------------------------
// Public types — match the existing TypeScript API
// ---------------------------------------------------------------------------

export type LiteParseInput = string | Buffer | Uint8Array;
export type OutputFormat = "json" | "text";

export interface LiteParseConfig {
  ocrLanguage: string;
  ocrEnabled: boolean;
  ocrServerUrl?: string;
  tessdataPath?: string;
  maxPages: number;
  targetPages?: string;
  dpi: number;
  outputFormat: OutputFormat;
  preserveVerySmallText: boolean;
  password?: string;
  quiet: boolean;
  numWorkers: number;
  cleanArtifacts: boolean;
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
}

export interface ParsedPage {
  pageNum: number;
  width: number;
  height: number;
  text: string;
  textItems: TextItem[];
}

export interface ParseResult {
  pages: ParsedPage[];
  text: string;
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
      tessdataPath: userConfig.tessdataPath,
      maxPages: userConfig.maxPages,
      targetPages: userConfig.targetPages,
      dpi: userConfig.dpi,
      outputFormat: userConfig.outputFormat,
      preserveVerySmallText: userConfig.preserveVerySmallText,
      password: userConfig.password,
      quiet: userConfig.quiet,
      numWorkers: userConfig.numWorkers,
      cleanArtifacts: userConfig.cleanArtifacts,
    };

    this._native = new native.LiteParse(nativeConfig);

    // Read back the resolved config from the native side
    const resolved = this._native.config;
    this._config = {
      ocrLanguage: resolved.ocrLanguage ?? "eng",
      ocrEnabled: resolved.ocrEnabled ?? true,
      ocrServerUrl: resolved.ocrServerUrl ?? undefined,
      tessdataPath: resolved.tessdataPath ?? undefined,
      maxPages: resolved.maxPages ?? 1000,
      targetPages: resolved.targetPages ?? undefined,
      dpi: resolved.dpi ?? 150,
      outputFormat: (resolved.outputFormat as OutputFormat) ?? "json",
      preserveVerySmallText: resolved.preserveVerySmallText ?? false,
      password: resolved.password ?? undefined,
      quiet: resolved.quiet ?? false,
      numWorkers: resolved.numWorkers ?? 1,
      cleanArtifacts: resolved.cleanArtifacts ?? true,
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
