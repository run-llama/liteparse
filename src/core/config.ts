import { LiteParseConfig } from './types.js';

export const DEFAULT_CONFIG: LiteParseConfig = {
  // OCR - defaults to in-process Tesseract for zero-setup experience
  // If ocrServerUrl is provided, uses HTTP OCR instead
  ocrLanguage: 'en',
  ocrEnabled: true,
  ocrServerUrl: undefined, // If set, uses HTTP OCR; otherwise uses Tesseract

  // Processing
  maxPages: 1000,
  targetPages: undefined,
  dpi: 150,

  // Output
  outputFormat: 'json',
  includeImages: true,
  includeCharts: true,

  // Features
  tableDetection: true,
  preciseBoundingBox: true,
  skipDiagonalText: false,
  preserveVerySmallText: false,
  preserveLayoutAlignmentAcrossPages: false,
};

export function mergeConfig(
  userConfig: Partial<LiteParseConfig>
): LiteParseConfig {
  return {
    ...DEFAULT_CONFIG,
    ...userConfig,
  };
}
