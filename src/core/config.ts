import { LiteParseConfig } from "./types.js";

export const DEFAULT_CONFIG: LiteParseConfig = {
  // OCR - defaults to in-process Tesseract for zero-setup experience
  // If ocrServerUrl is provided, uses HTTP OCR instead
  ocrLanguage: "en",
  ocrEnabled: true,
  ocrServerUrl: undefined, // If set, uses HTTP OCR; otherwise uses Tesseract
  numWorkers: 4, // Number of pages to OCR in parallel

  // Processing
  maxPages: 1000,
  targetPages: undefined,
  dpi: 150,

  // Output
  outputFormat: "json",

  // Features
  preciseBoundingBox: true,
  preserveVerySmallText: false,
  preserveLayoutAlignmentAcrossPages: false,
  password: undefined,
};

export function mergeConfig(userConfig: Partial<LiteParseConfig>): LiteParseConfig {
  return {
    ...DEFAULT_CONFIG,
    ...userConfig,
  };
}
