// Native binary loader - tries platform-specific packages, falls back to local .node file.
//
// In production: the correct @llamaindex/liteparse-<platform> optional dependency
// provides the .node binary. During development: `napi build` places it alongside package.json.

import { createRequire } from "node:module";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));

interface NativeBindings {
  LiteParse: new (config?: LiteParseNativeConfig) => LiteParseNative;
  searchItems(
    items: NativeTextItem[],
    phrase: string,
    caseSensitive?: boolean | null,
  ): NativeTextItem[];
}

export interface LiteParseNativeConfig {
  ocrLanguage?: string;
  ocrEnabled?: boolean;
  ocrServerUrl?: string;
  ocrServerHeaders?: Record<string, string>;
  tessdataPath?: string;
  maxPages?: number;
  targetPages?: string;
  extractScreenshots?: boolean;
  continueOnPageError?: boolean;
  dpi?: number;
  outputFormat?: string;
  imageMode?: string;
  extractImages?: boolean;
  imageOutputDir?: string;
  extractLinks?: boolean;
  keepHeadersFooters?: boolean;
  extractAnnotations?: boolean;
  extractFormFields?: boolean;
  extractStructureTree?: boolean;
  extractBlocks?: boolean;
  extractXfaPackets?: boolean;
  extractDocumentMetadata?: boolean;
  extractContentBounds?: boolean;
  detectScreenshotRects?: boolean;
  renderFormFields?: boolean;
  preserveVerySmallText?: boolean;
  password?: string;
  quiet?: boolean;
  numWorkers?: number;
  ocrFailureFatal?: boolean;
  ocrHedgeDelaysMs?: number[];
  emitWordBoxes?: boolean;
  extractTextMetadata?: boolean;
  cropBox?: NativeCropBox;
  skipDiagonalText?: boolean;
  includeComplexity?: boolean;
  extractVectorGraphics?: boolean;
}

export interface NativeCropBox {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export interface NativeWordBox {
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NativeTextItem {
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  fontName?: string;
  fontSize?: number;
  fontHeight?: number;
  fontAscent?: number;
  fontDescent?: number;
  fontWeight?: number;
  textWidth?: number;
  fontIsBuggy?: boolean;
  mcid?: number;
  fillColor?: string;
  strokeColor?: string;
  charCodes?: number[];
  trailingSpaceGenerated?: boolean;
  confidence?: number;
  rotation?: number;
  words?: NativeWordBox[];
}

export interface NativeGraphic {
  kind: string;
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

export interface NativePageInput {
  pageNumber: number;
  pageWidth: number;
  pageHeight: number;
  textItems: NativeTextItem[];
  graphics?: NativeGraphic[];
}

export interface NativeRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NativeParsedPage {
  pageNum: number;
  width: number;
  height: number;
  contentBounds?: NativeRect;
  text: string;
  markdown: string;
  textItems: NativeTextItem[];
  complexity?: NativePageComplexityStats;
  vectorGraphics?: NativeVectorGraphics;
  annotations?: NativeDocumentAnnotation[];
  formFields?: NativeFormField[];
  structureTree?: NativeStructureTree;
  blocks?: NativeLayoutBlock[];
}

export interface NativeLayoutCell {
  text: string;
  bbox?: NativeRect;
  textItemIndices: number[];
}

export interface NativeLayoutBlock {
  kind: string;
  textItemIndices: number[];
  text?: string;
  level?: number;
  bold?: boolean;
  italic?: boolean;
  ordered?: boolean;
  marker?: string;
  lines?: string[];
  lang?: string;
  header?: NativeLayoutCell[];
  rows?: NativeLayoutCell[][];
  id?: string;
  format?: string;
  bbox?: NativeRect;
}

export interface NativeStructureAttribute {
  name: string;
  booleanValue?: boolean;
  numberValue?: number;
  stringValue?: string;
}

export interface NativeStructureTree {
  roots: NativeStructureTreeElement[];
}

export interface NativeStructureTreeElement {
  elementType: string;
  id?: string;
  actualText?: string;
  altText?: string;
  title?: string;
  attributes: NativeStructureAttribute[];
  markedContentIds: number[];
  children: NativeStructureTreeElement[];
  annotations: NativeDocumentAnnotation[];
}

export interface NativeVectorGraphics {
  shapes: Array<{
    bbox: { x: number; y: number; width: number; height: number };
    stroke: boolean;
    strokeColor?: string;
    fill: boolean;
    fillColor?: string;
    hasCurve: boolean;
  }>;
  lines: Array<{
    x1: number; y1: number; x2: number; y2: number;
    stroke: boolean; strokeWidth?: number; strokeColor?: string;
    fill: boolean; fillColor?: string;
  }>;
}

export interface NativeAnnotationRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NativeDocumentAnnotation {
  subtype: string;
  contents?: string;
  created?: string;
  modified?: string;
  title?: string;
  rect?: NativeAnnotationRect;
  quadpointRects: NativeAnnotationRect[];
  uri?: string;
}

export interface NativeFormField {
  id: string;
  fieldType: string;
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
  rect?: NativeAnnotationRect;
  options: string[];
  selectedOptions: string[];
}

export interface NativeExtractedImage {
  id: string;
  name: string;
  path?: string;
  page: number;
  bbox: { x: number; y: number; width: number; height: number };
  width: number;
  height: number;
  rotation: number;
  format: string;
  duplicateOf?: string;
  bytes: Buffer;
}

export interface NativeParseResult {
  totalPages: number;
  pages: NativeParsedPage[];
  pageErrors: Array<{ pageNum: number; message: string }>;
  text: string;
  images: NativeExtractedImage[];
  screenshots: NativeScreenshotResult[];
  imageErrorCount: number;
  formType?: number;
  creator?: string;
  producer?: string;
  docMeta?: NativeDocumentMetadata;
  xfaPackets?: NativeXfaPacket[];
}

export interface NativeDocumentMetadata {
  creationDate?: string;
  modDate?: string;
  fileVersion?: number;
  isEncrypted?: boolean;
  securityHandlerRevision?: number;
  permissions?: number;
  eofSectionCount?: number;
  startxrefCount?: number;
  trailerIdPairDiffers?: boolean;
  rawFileSize?: number;
  xmp?: string;
  xmpTruncated?: boolean;
  signatureCount?: number;
  signatureByteRangeReachesEof?: boolean;
}

export interface NativeXfaPacket {
  index: number;
  name?: string;
  contentLength: number;
  content?: string;
}

export interface NativeScreenshotResult {
  pageNum: number;
  width: number;
  height: number;
  imageBuffer: Buffer;
  isSolidFill: boolean;
  rects: NativeScreenshotRect[];
}

export interface NativeScreenshotRect {
  x: number;
  y: number;
  width: number;
  height: number;
  color: string;
  isLine: boolean;
}

export interface NativeLayoutComplexityStats {
  columnCount: number;
  ruledTableCount: number;
  ruledTableCoverage: number;
  textTableRunCount: number;
  figureCount: number;
  figureCoverage: number;
  isComplex: boolean;
  reasons: string[];
}

export interface NativePageComplexityStats {
  pageNumber: number;
  textLength: number;
  textCoverage: number;
  hasSubstantialImages: boolean;
  imageBlockCount: number;
  imageCoverage: number;
  largestImageCoverage: number;
  fullPageImage: boolean;
  uncoveredVectorArea?: number;
  isGarbled: boolean;
  pageArea: number;
  needsOcr: boolean;
  reasons: string[];
  layout?: NativeLayoutComplexityStats;
}

export interface NativeParseBatch {
  startPage: number;
  endPage: number;
  result: NativeParseResult;
}

export interface NativeParseSession {
  nextBatch(): Promise<NativeParseBatch | null>;
  close(): Promise<void>;
  readonly totalPages: number;
}

export interface LiteParseNative {
  parse(input: string | Buffer): Promise<NativeParseResult>;
  openBatchSession(
    input: string | Buffer,
    batchSize?: number,
  ): Promise<NativeParseSession>;
  parsePages(pages: NativePageInput[]): NativeParseResult;
  isComplex(input: string | Buffer): Promise<NativePageComplexityStats[]>;
  screenshot(
    input: string | Buffer,
    pageNumbers?: number[] | null,
  ): Promise<NativeScreenshotResult[]>;
  format(result: NativeParseResult): string;
  readonly config: LiteParseNativeConfig;
}

function loadNative(): NativeBindings {
  // Platform-specific package names generated by napi-rs
  const triples: Record<string, string> = {
    "darwin-x64": "@llamaindex/liteparse-darwin-x64",
    "darwin-arm64": "@llamaindex/liteparse-darwin-arm64",
    "linux-x64-gnu": "@llamaindex/liteparse-linux-x64-gnu",
    "linux-x64-musl": "@llamaindex/liteparse-linux-x64-musl",
    "linux-arm64-gnu": "@llamaindex/liteparse-linux-arm64-gnu",
    "linux-arm64-musl": "@llamaindex/liteparse-linux-arm64-musl",
    "win32-x64-msvc": "@llamaindex/liteparse-win32-x64-msvc",
    "win32-arm64-msvc": "@llamaindex/liteparse-win32-arm64-msvc",
  };

  // Try platform-specific package first
  const platform = process.platform;
  const arch = process.arch;

  const candidates: string[] = [];
  if (platform === "linux") {
    // Try gnu first, then musl
    candidates.push(`${platform}-${arch}-gnu`);
    candidates.push(`${platform}-${arch}-musl`);
  } else if (platform === "win32") {
    candidates.push(`${platform}-${arch}-msvc`);
  } else {
    candidates.push(`${platform}-${arch}`);
  }

  for (const key of candidates) {
    const pkg = triples[key];
    if (pkg) {
      try {
        return require(pkg);
      } catch {
        // Not installed, try next
      }
    }
  }

  // Fallback: local .node file (development builds)
  // Try several paths since __dirname may be dist/ or dist/src/
  const searchDirs = [__dirname, join(__dirname, ".."), join(__dirname, "..", "..")];
  // Try full triple names (e.g. liteparse.linux-x64-gnu.node) and simple name
  const fileNames = [
    ...candidates.map((c) => `liteparse.${c}.node`),
    `liteparse.${platform}-${arch}.node`,
    "liteparse.node",
  ];
  for (const dir of searchDirs) {
    for (const fileName of fileNames) {
      try {
        return require(join(dir, fileName));
      } catch {
        // try next
      }
    }
  }

  throw new Error(
    `Failed to load native module for ${platform}-${arch}. ` +
      `Ensure the correct optional dependency is installed.`,
  );
}

export const native = loadNative();
