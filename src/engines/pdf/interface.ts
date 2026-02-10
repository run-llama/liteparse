import { TextItem } from '../../core/types.js';

export interface PdfEngine {
  name: string;
  loadDocument(filePath: string): Promise<PdfDocument>;
  extractPage(doc: PdfDocument, pageNum: number): Promise<PageData>;
  extractAllPages(
    doc: PdfDocument,
    maxPages?: number,
    targetPages?: string
  ): Promise<PageData[]>;
  renderPageImage(
    doc: PdfDocument,
    pageNum: number,
    dpi: number
  ): Promise<Buffer>;
  close(doc: PdfDocument): Promise<void>;
}

export interface PdfDocument {
  numPages: number;
  data: Uint8Array;
  metadata?: any;
}

export interface PageData {
  pageNum: number;
  width: number;
  height: number;
  textItems: TextItem[];
  paths: Path[];
  images: Image[];
  annotations?: Annotation[];
}

export interface Path {
  type: 'rectangle' | 'line' | 'curve';
  points: number[][];
  color?: string;
  width?: number;
}

export interface Image {
  x: number;
  y: number;
  width: number;
  height: number;
  data?: Buffer;
  coords?: { x: number; y: number; w: number; h: number };
  scaleFactor?: number;
  originalOrientationAngle?: number;
  type?: string;
  ocrRaw?: EasyOcrResultLine[];
  ocrParsed?: Array<{
    x: number;
    y: number;
    w: number;
    h: number;
    confidence: number;
    text: string;
  }>;
}

// OCR result line: [coordinates (4 points with x,y), text, confidence]
export type EasyOcrResultLine = [
  [[number, number], [number, number], [number, number], [number, number]],
  string,
  string | number
];

export interface Annotation {
  type: string;
  subtype?: string;
  url?: string;
  rect: number[];
}
