export type OutputFormat = 'json' | 'text';

export interface LiteParseConfig {
  // OCR
  ocrLanguage: string | string[];
  ocrEnabled: boolean;

  // HTTP OCR Server (optional - if not provided, uses Tesseract)
  ocrServerUrl?: string;

  // Processing
  maxPages: number;
  targetPages?: string;
  dpi: number;

  // Output
  outputFormat: OutputFormat;
  includeImages: boolean;
  includeCharts: boolean;

  // Features
  tableDetection: boolean;
  preciseBoundingBox: boolean;
  skipDiagonalText: boolean;
  preserveVerySmallText: boolean;
  preserveLayoutAlignmentAcrossPages: boolean;
}

export interface TextItem {
  str: string;
  x: number;
  y: number;
  width: number;
  height: number;
  w: number;  // Alias for width
  h: number;  // Alias for height
  fontName?: string;
  fontSize?: number;
  r?: number; // Rotation angle in degrees (0, 90, 180, 270)
  rx?: number; // Rotated x coordinate
  ry?: number; // Rotated y coordinate
  markup?: MarkupData;
  vgap?: boolean;
  isPlaceholder?: boolean;
}

export interface MarkupData {
  highlight?: string;
  underline?: boolean;
  squiggly?: boolean;
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
  snap?: 'left' | 'right' | 'center';
  leftAnchor?: string;
  rightAnchor?: string;
  centerAnchor?: string;
  isDup?: boolean;
  rendered?: boolean;
  shouldSpace?: number;
  forceUnsnapped?: boolean;
  rotated?: boolean;
  d?: number; // Delta for rotation handling
}

export interface Coordinates {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface OcrData {
  x: number;
  y: number;
  w: number;
  h: number;
  confidence: number;
  text: string;
}

export interface BoundingBox {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

export interface ParsedPage {
  pageNum: number;
  width: number;
  height: number;
  text: string;
  textItems: TextItem[];
  boundingBoxes?: BoundingBox[];
  tables?: any[];
  images?: any[];
}

export interface ParseResult {
  pages: ParsedPage[];
  text: string;
  json?: any;
}

export interface ScreenshotResult {
  pageNum: number;
  width: number;
  height: number;
  imageBuffer: Buffer;
  imagePath?: string;
}
