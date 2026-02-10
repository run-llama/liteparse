import { Image, EasyOcrResultLine } from '../engines/pdf/interface.js';

export interface OcrBlock {
  c: string; // Content/text
  x: number; // X coordinate in page space
  rx: number; // Raw X coordinate
  y: number; // Y coordinate in page space
  ry: number; // Raw Y coordinate
  w: number; // Width in page space
  rw: number; // Raw width
  h: number; // Height in page space
  rh: number; // Raw height
  confidence: string | number;
  fromOcr: boolean;
}

/**
 * Parse EasyOCR blocks from image with OCR data
 * Converts OCR bounding boxes from image space to page space
 */
export function parseEasyOCRBlocks(image: Image): OcrBlock[] {
  if (!image || !image.ocrRaw?.length) {
    return [];
  }

  const blocks: OcrBlock[] = [];

  // Calculate ratio to convert from OCR image coordinates to page coordinates
  const coords = image.coords || { x: 0, y: 0, w: image.width, h: image.height };
  const xRatio = image.width / coords.w;
  const yRatio = image.height / coords.h;

  for (const line of image.ocrRaw) {
    // line format: [[[x1, y1], [x2, y2], [x3, y3], [x4, y4]], 'text', confidence]
    const [x1, y1] = line[0][0];
    const [x2, y2] = line[0][2]; // Use opposite corner for bbox
    const text = line[1];
    const confidence = line[2];

    // Convert the bounding box to page space
    const block: OcrBlock = {
      c: text,
      x: x1 / xRatio + coords.x,
      rx: Math.round(x1 / (image.scaleFactor || 1)),
      y: y1 / yRatio + coords.y,
      ry: Math.round(y1 / (image.scaleFactor || 1)),
      w: Math.abs(x2 - x1) / xRatio,
      rw: Math.round(Math.abs(x2 - x1) / (image.scaleFactor || 1)),
      h: Math.abs(y2 - y1) / yRatio,
      rh: Math.round(Math.abs(y2 - y1) / (image.scaleFactor || 1)),
      confidence,
      fromOcr: true,
    };
    blocks.push(block);
  }

  return blocks;
}

/**
 * Parse EasyOCR stdout result into structured format
 */
export function easyOcrResultLinesToList(
  stdOutResult?: string
): EasyOcrResultLine[] {
  if (!stdOutResult?.length) {
    return [];
  }

  const lines = stdOutResult.split('\n');
  const blocks: EasyOcrResultLine[] = [];

  for (const line of lines) {
    if (!line.trim()) {
      continue;
    }

    // Parse line format: ([[x1, y1], [x2, y2], [x3, y3], [x4, y4]], 'text', confidence)
    const ocrMatch = line.match(
      /\[\[(.*?), (.*?)\], \[(.*?), (.*?)\], \[(.*?), (.*?)\], \[(.*?), (.*?)\]\], ['"](.*?)['"], (.*?)\)$/
    );

    if (!ocrMatch) {
      continue;
    }

    const x1 = Number(ocrMatch[1]);
    const y1 = Number(ocrMatch[2]);
    const x2 = Number(ocrMatch[3]);
    const y2 = Number(ocrMatch[4]);
    const x3 = Number(ocrMatch[5]);
    const y3 = Number(ocrMatch[6]);
    const x4 = Number(ocrMatch[7]);
    const y4 = Number(ocrMatch[8]);
    const text = ocrMatch[9];
    const confidence = ocrMatch[10];

    blocks.push([
      [
        [x1, y1],
        [x2, y2],
        [x3, y3],
        [x4, y4],
      ],
      text,
      confidence,
    ]);
  }

  return blocks;
}
