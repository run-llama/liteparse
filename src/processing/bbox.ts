import {
  TextItem,
  BoundingBox,
  ProjectionTextBox,
  OcrData,
  LiteParseConfig,
} from '../core/types.js';
import { PageData, Image } from '../engines/pdf/interface.js';
import { parseEasyOCRBlocks, OcrBlock } from './ocrUtils.js';

const OCR_CONFIDENCE_THRESHOLD = 0.1;

/**
 * Minimum overlap ratio (0-1) required to consider an OCR block as duplicate of existing text.
 * An OCR block is filtered out if:
 * - Total overlap with all text items covers more than this ratio of the OCR block area
 * - OR the OCR block covers more than this ratio of any single text item
 */
const OCR_OVERLAP_THRESHOLD = 0.5;

/**
 * Maximum number of embedded images to process for OCR per page.
 * Keeps the largest images when limit is exceeded.
 */
const MAX_IMAGES_PER_PAGE = 10;

/**
 * Minimum image dimensions for OCR processing
 */
const MIN_IMAGE_DIMENSION = 12;
const MIN_IMAGE_AREA = 200;

/**
 * Minimum rendered image dimensions for OCR processing
 */
const MIN_RENDERED_DIMENSION = 6;
const MIN_RENDERED_AREA = 200;

/**
 * Filters images that should not be OCR'd based on various criteria.
 * Returns the filtered array of images that should be processed.
 *
 * Ported from llamaparse's filterImagesForOCR function.
 */
export function filterImagesForOCR(
  images: Image[],
  page: { width: number; height: number }
): Image[] {
  // Filter images that start with g_ or pattern_ (generated/pattern images)
  let filtered = images.filter(
    (image) =>
      !image.type?.startsWith('g_') && !image.type?.startsWith('pattern_')
  );

  // Limit to max images per page, keeping the largest ones
  if (filtered.length > MAX_IMAGES_PER_PAGE) {
    filtered.sort((a, b) => b.width * b.height - a.width * a.height);
    filtered = filtered.slice(0, MAX_IMAGES_PER_PAGE);
  }

  // Apply additional filtering criteria
  filtered = filtered.filter((image) => {
    // Ignore layout extracted images
    if (image.type?.includes('layout_')) {
      return false;
    }

    // Get image coords (use image dimensions if coords not set)
    const coords = image.coords || {
      x: image.x,
      y: image.y,
      w: image.width,
      h: image.height,
    };

    // Skip images that are out of viewport
    if (
      coords.x + coords.w < 0 || // left of page
      coords.y + coords.h < 0 || // above page
      coords.x > page.width || // right of page
      coords.y > page.height // below page
    ) {
      return false;
    }

    // Skip small images (raw dimensions)
    if (
      image.width < MIN_IMAGE_DIMENSION ||
      image.height < MIN_IMAGE_DIMENSION ||
      image.width * image.height < MIN_IMAGE_AREA
    ) {
      return false;
    }

    // Skip images that render too small in the viewport
    if (
      coords.w < MIN_RENDERED_DIMENSION ||
      coords.h < MIN_RENDERED_DIMENSION ||
      coords.w * coords.h < MIN_RENDERED_AREA
    ) {
      return false;
    }

    return true;
  });

  return filtered;
}

/**
 * Checks if two bounding boxes overlap and returns the overlap area.
 */
function getOverlapArea(
  box1: { x: number; y: number; w: number; h: number },
  box2: { x: number; y: number; w: number; h: number }
): number {
  const left = Math.max(box1.x, box2.x);
  const right = Math.min(box1.x + box1.w, box2.x + box2.w);
  const top = Math.max(box1.y, box2.y);
  const bottom = Math.min(box1.y + box1.h, box2.y + box2.h);

  if (left >= right || top >= bottom) {
    return 0;
  }

  return (right - left) * (bottom - top);
}

/**
 * Filters out OCR blocks that significantly overlap with already-extracted text items.
 * This prevents duplicate text when both document text extraction and OCR detect the same content.
 * Prefers document-extracted text over OCR text.
 *
 * An OCR block is rejected if:
 * 1. The total overlap with all text items covers more than 50% of the OCR block area
 * 2. OR the OCR block covers more than 50% of any single text item's area
 */
function filterOcrBlocksOverlappingWithText(
  ocrBlocks: OcrBlock[],
  textItems: Array<{ x: number; y: number; w: number; h: number }>
): OcrBlock[] {
  if (!textItems.length || !ocrBlocks.length) {
    return ocrBlocks;
  }

  return ocrBlocks.filter((ocrBlock) => {
    const ocrBox = {
      x: ocrBlock.x,
      y: ocrBlock.y,
      w: ocrBlock.w,
      h: ocrBlock.h,
    };
    const ocrArea = ocrBlock.w * ocrBlock.h;

    if (ocrArea <= 0) {
      return false;
    }

    let totalOverlapArea = 0;

    // Check overlap with each text item
    for (const textItem of textItems) {
      const textBox = {
        x: textItem.x,
        y: textItem.y,
        w: textItem.w,
        h: textItem.h,
      };
      const textItemArea = textItem.w * textItem.h;

      const overlapArea = getOverlapArea(ocrBox, textBox);

      if (overlapArea > 0) {
        // Accumulate total overlap for condition 1
        totalOverlapArea += overlapArea;

        // Condition 2: Reject if OCR block covers more than 50% of any single text item
        if (
          textItemArea > 0 &&
          overlapArea / textItemArea >= OCR_OVERLAP_THRESHOLD
        ) {
          return false;
        }
      }
    }

    // Condition 1: Reject if total overlap covers more than 50% of the OCR block
    const totalOverlapRatio = totalOverlapArea / ocrArea;
    if (totalOverlapRatio >= OCR_OVERLAP_THRESHOLD) {
      return false;
    }

    return true;
  });
}

/**
 * Build projection text boxes from page data, including OCR results
 * This is the complete implementation from buildBbox.ts
 */
export function buildBbox(
  pageData: PageData,
  config: LiteParseConfig
): ProjectionTextBox[] {
  const lines: ProjectionTextBox[] = [];

  // Process all extracted text items
  for (const item of pageData.textItems) {
    const line: ProjectionTextBox = {
      x: item.x,
      y: item.y,
      rx: item.rx || 0,
      ry: item.ry || 0,
      w: Math.round(item.w || item.width),
      h: Math.round(item.h || item.height),
      r: item.r || 0,
      str: item.str,
      strLength: [...item.str].length, // Handle multi-byte characters correctly
      pageBbox: {
        x: item.x,
        y: item.y,
        w: item.w || item.width,
        h: item.h || item.height,
      },
      vgap: item.vgap,
      isPlaceholder: item.isPlaceholder,
    };

    lines.push(line);
  }

  // Process OCR data if images are present
  if (pageData.images.length && config.ocrEnabled) {
    // Filter images that should be processed for OCR
    const imagesToProcess = filterImagesForOCR(pageData.images, {
      width: pageData.width,
      height: pageData.height,
    });

    // Collect text item bounding boxes for overlap checking
    const textItemBoxes = pageData.textItems.map((item) => ({
      x: item.x,
      y: item.y,
      w: item.w || item.width,
      h: item.h || item.height,
    }));

    for (const image of imagesToProcess) {

      // Parse OCR blocks from image
      let ocrData = parseEasyOCRBlocks(image);

      // Filter by confidence threshold
      ocrData = ocrData.filter(
        (block) => parseFloat(block.confidence.toString()) >= OCR_CONFIDENCE_THRESHOLD
      );

      // Filter out OCR blocks that overlap with already-extracted text
      ocrData = filterOcrBlocksOverlappingWithText(ocrData, textItemBoxes);

      const ocrParsed: OcrData[] = [];
      for (const block of ocrData) {
        const confidenceRounded =
          Math.round(parseFloat(block.confidence.toString()) * 1000) / 1000;

        const line: ProjectionTextBox = {
          fromOCR: true,
          x: block.x,
          y: block.y,
          w: block.w,
          h: block.h,
          r: image.originalOrientationAngle || 0,
          str: block.c,
          strLength: [...block.c].length,
          pageBbox: {
            x: block.x,
            y: block.y,
            w: block.w,
            h: block.h,
          },
        };
        lines.push(line);

        ocrParsed.push({
          x: block.rx,
          y: block.ry,
          w: block.rw,
          h: block.rh,
          confidence: confidenceRounded,
          text: block.c,
        });
      }

      if (ocrParsed.length) {
        image.ocrParsed = ocrParsed;
      }
    }
  }

  return lines;
}

/**
 * Build bounding boxes from text items
 */
export function buildBoundingBoxes(textItems: TextItem[]): BoundingBox[] {
  const bboxes: BoundingBox[] = [];

  for (const item of textItems) {
    if (item.str.trim() === '') {
      continue;
    }

    bboxes.push({
      x1: item.x,
      y1: item.y,
      x2: item.x + (item.w || item.width),
      y2: item.y + (item.h || item.height),
    });
  }

  return bboxes;
}
