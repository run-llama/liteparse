import { strToSubscriptString, strToPostScript } from "./textUtils.js";
import { buildBbox } from "./bbox.js";
import { cleanRawText } from "./cleanText.js";
import { ProjectionTextBox, Coordinates, LiteParseConfig, ParsedPage } from "../core/types.js";
import { PageData } from "../engines/pdf/interface.js";

import { applyMarkupTags } from "./markupUtils.js";
import { GridDebugLogger, createGridDebugLogger } from "./gridDebugLogger.js";
import { renderAllVisualizations } from "./gridVisualizer.js";

// Minimum spaces between unsnapped bboxes (likely justified text
const FLOATING_SPACES = 2;
// Minimum spaces between snapped columns
const COLUMN_SPACES = 4;

// --- Flowing text detection thresholds ---
// Max total anchors (left+right+center) before block is considered structured
const FLOWING_MAX_TOTAL_ANCHORS = 4;
// Max left anchors before block is considered structured
const FLOWING_MAX_LEFT_ANCHORS = 3;
// Minimum non-empty lines required to classify a block
const FLOWING_MIN_LINES = 3;
// Fraction of page width a line must span to count as "wide"
const FLOWING_WIDE_LINE_RATIO = 0.5;
// Fraction of lines that must be wide for a block to be flowing
const FLOWING_WIDE_LINE_THRESHOLD = 0.6;
// Multiplier on median char width for column gap detection
const FLOWING_COLUMN_GAP_MULTIPLIER = 4;
// Minimum items on a line to be classified as flowing in per-line detection
const FLOWING_MIN_LINE_ITEMS = 3;
// Height multiplier for word-break space threshold in flowing text
const FLOWING_SPACE_HEIGHT_RATIO = 0.15;
// Minimum absolute space threshold in flowing text
const FLOWING_SPACE_MIN_THRESHOLD = 0.3;
// Maximum indent (in character widths) for flowing text
const FLOWING_MAX_INDENT = 8;

type Snap = {
  bbox: ProjectionTextBox;
  lineIndex: number;
  boxIndex: number;
};

type Anchor = {
  [key: number]: ProjectionTextBox[];
};

type Anchors = {
  anchorLeft: Anchor;
  anchorRight: Anchor;
  anchorCenter: Anchor;
};

type ForwardAnchor = { [key: string]: number };

type PrevAnchors = {
  forwardAnchorLeft: ForwardAnchor;
  forwardAnchorCenter: ForwardAnchor;
  forwardAnchorRight: ForwardAnchor;
};

type PageForwardAnchors = {
  left: ForwardAnchor;
  right: ForwardAnchor;
  center: ForwardAnchor;
  floating: ForwardAnchor;
};

type SnapMaps = {
  left: number[];
  right: number[];
  center: number[];
  floating: number[];
};

type LineRange = {
  start: number;
  end: number;
};

type TextBoxSize = {
  width: number;
  height: number;
};

type LineMetrics = {
  top: number;
  bottom: number;
  height: number;
};

function roundAnchor(anchor: number): number {
  // group anchor x-coord by nearest 1/4 unit
  return Math.round(anchor * 4) / 4;
}

function getRepresentativeLineMetrics(
  line: ProjectionTextBox[],
  globalMedianHeight: number
): LineMetrics {
  const minRepresentativeHeight = globalMedianHeight * 0.5;
  const representativeItems = line.filter((bbox) => bbox.h >= minRepresentativeHeight);
  const items = representativeItems.length > 0 ? representativeItems : line;

  const top = Math.min(...items.map((bbox) => bbox.y));
  const bottom = Math.max(...items.map((bbox) => bbox.y + bbox.h));

  return {
    top,
    bottom,
    height: bottom - top,
  };
}

// 2pt @ PDF 72 DPI -> 8px @ 300DPI
const SMALL_FONT_SIZE_THRESHOLD = 2;

function isSmallTextLine(line: ProjectionTextBox[]): boolean {
  // check for line where >50% of the text is very small
  const smallText = line.filter((item) => item.h < SMALL_FONT_SIZE_THRESHOLD);
  if (smallText.length / line.length > 0.5) {
    return true;
  }
  return false;
}

function filterUnprojectableText(
  config: LiteParseConfig,
  line: ProjectionTextBox[]
): ProjectionTextBox[] {
  // Filter text items that would break projection (e.g., very small text)
  if (line.length === 0) {
    return line;
  }

  let filteredLine = line;
  if (!config.preserveVerySmallText && isSmallTextLine(line)) {
    // remove very small text lines
    filteredLine = filteredLine.filter((item) => item.h >= SMALL_FONT_SIZE_THRESHOLD);
  }
  return filteredLine;
}

function canSnapLine(config: LiteParseConfig, line: ProjectionTextBox[]): boolean {
  // force lines that will likely break projection to be unsnapped floating text
  // currently this includes:
  //   - lines of entirely small text
  //
  // NOTE: this assumes undesirable text has already been filtered before projection
  // (i.e. parse mode based removal of text should be done before this in filterUnprojectableText())
  if (line.length === 0) {
    return true;
  }

  if (!config.preserveVerySmallText && isSmallTextLine(line)) {
    return false;
  }
  return true;
}

function fixSparseBlocks(blocks: LineRange[], rawLines: string[]) {
  // compress whitespace in blocks containing very sparse lines (>80% whitespace)
  const regexp = new RegExp(`\\s{${COLUMN_SPACES},}`, "g");
  for (const block of blocks) {
    let total = 0;
    let whitespace = 0;
    for (let i = block.start; i < block.end; ++i) {
      if (!rawLines[i]) {
        continue;
      }
      rawLines[i] = rawLines[i].trimEnd();
      const line = rawLines[i];
      if (line.length === 0) {
        continue;
      }
      total += line.length;
      whitespace += line.match(/\s/g)?.length || 0;
    }
    if (total >= 500 && whitespace / total > 0.8) {
      for (let i = block.start; i < block.end; ++i) {
        const line = rawLines[i];
        if (!line || line.length === 0) {
          continue;
        }
        rawLines[i] = line.replace(regexp, " ".repeat(FLOATING_SPACES));
      }
    }
  }
}

function extractAnchorsPointsFromLines(lines: ProjectionTextBox[][], page: PageData): Anchors {
  const pageHeight = page.height;

  const anchorLeft: Anchor = {};
  const anchorRight: Anchor = {};
  const anchorCenter: Anchor = {};

  for (const line of lines) {
    for (const bbox of line) {
      let anchor = roundAnchor(bbox.x);
      if (!anchorLeft[anchor]) {
        anchorLeft[anchor] = [];
      }
      anchorLeft[anchor].push(bbox);

      anchor = roundAnchor(bbox.x + bbox.w);
      if (!anchorRight[anchor]) {
        anchorRight[anchor] = [];
      }
      anchorRight[anchor].push(bbox);

      const center = Math.round(bbox.x + bbox.w / 2);
      if (!anchorCenter[center]) {
        anchorCenter[center] = [];
      }
      anchorCenter[center].push(bbox);
    }
  }

  function deltaMin(collection: Anchor, delta: number) {
    for (const anchor in collection) {
      const maxDelta = pageHeight * delta;
      for (let i = 0; i < collection[anchor].length; i++) {
        let shouldKeep = false;
        if (i > 0) {
          if (collection[anchor][i].y - collection[anchor][i - 1].y < maxDelta) {
            shouldKeep = true;
          }
        }
        if (i < collection[anchor].length - 1) {
          if (collection[anchor][i + 1].y - collection[anchor][i].y < maxDelta) {
            shouldKeep = true;
          }
        }

        if (!shouldKeep) {
          collection[anchor].splice(i--, 1);
        }
      }
    }
  }

  // Group nearby anchors FIRST to merge items at similar positions
  // This ensures deltaMin operates on combined groups, not isolated items
  group(anchorLeft);
  group(anchorRight);
  group(anchorCenter);

  deltaMin(anchorRight, 0.17);
  deltaMin(anchorLeft, 0.2);
  deltaMin(anchorCenter, 0.05);

  function intercept(collection: Anchor) {
    for (const anchor in collection) {
      let shouldKeep = false;
      for (let i = 0; i < collection[anchor].length; i++) {
        if (i > 0) {
          let intercept = false;
          // check intercept
          const a1 = collection[anchor][i - 1];
          const a2 = collection[anchor][i];
          for (const line of lines) {
            if (line.length > 0 && line[0].y > a1.y && line[0].y < a2.y) {
              for (const item of line) {
                if (item.x < parseInt(anchor) && item.x + item.w > parseInt(anchor)) {
                  intercept = true;
                  break;
                }
              }
              if (intercept) {
                break;
              }
            }
          }
          if (!intercept) {
            shouldKeep = true;
            break;
          }
        }
      }
      if (!shouldKeep) {
        delete collection[anchor];
      }
    }
  }

  intercept(anchorRight);
  intercept(anchorLeft);
  intercept(anchorCenter);

  function group(collection: Anchor) {
    // Sort anchors to process them in order
    const sortedAnchors = Object.keys(collection)
      .map((a) => parseFloat(a))
      .sort((a, b) => a - b);

    // Merge nearby anchors within a tolerance
    // Use 2 units as tolerance - this catches columns that are close but not exactly aligned
    const MERGE_TOLERANCE = 2;

    for (let i = 0; i < sortedAnchors.length; i++) {
      const anchor = sortedAnchors[i];
      if (!(anchor in collection)) continue; // Already merged

      // Look for nearby anchors to merge into this one or merge this into
      for (let j = i + 1; j < sortedAnchors.length; j++) {
        const nextAnchor = sortedAnchors[j];
        if (!(nextAnchor in collection)) continue;

        // Stop if we're beyond the tolerance
        if (nextAnchor - anchor > MERGE_TOLERANCE) break;

        // Merge the smaller anchor into the larger one
        if (collection[nextAnchor].length > collection[anchor].length) {
          collection[nextAnchor].push(...collection[anchor]);
          delete collection[anchor];
          break; // This anchor is gone, move to next
        } else {
          collection[anchor].push(...collection[nextAnchor]);
          delete collection[nextAnchor];
        }
      }
    }
  }

  function anyAnchorSurvived(bbox: ProjectionTextBox) {
    return (
      roundAnchor(bbox.x) in anchorLeft ||
      roundAnchor(bbox.x + bbox.w) in anchorRight ||
      Math.round(bbox.x + bbox.w / 2) in anchorCenter
    );
  }

  // Try seeing if a floating bbox would align well with a surviving anchor on a line immediately above or below it
  function tryAlignFloating(
    collection: Anchor,
    ANCHOR_MARGIN: number,
    refXFromBbox: (bbox: ProjectionTextBox) => number,
    anchorValFromBbox: (bbox: ProjectionTextBox) => number
  ) {
    for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
      const line = lines[lineIndex];
      for (const bbox of line) {
        // Only consider floating bboxes
        if (anyAnchorSurvived(bbox)) {
          continue;
        }
        // Check the lines before and after
        const candidateLines: ProjectionTextBox[][] = [];
        if (lineIndex > 0) {
          candidateLines.push(lines[lineIndex - 1]);
        }
        if (lineIndex < lines.length - 1) {
          candidateLines.push(lines[lineIndex + 1]);
        }

        // Check candidate lines for:
        // Possible alignment
        // Being within the margin
        // Being the closest of the candidates
        let candidateAnchor: string = "";
        let prevDiff = ANCHOR_MARGIN + 1;
        for (const candLine of candidateLines) {
          for (const candBBox of candLine) {
            const candAnchorVal = anchorValFromBbox(candBBox);
            if (!(candAnchorVal in collection)) {
              continue;
            }
            const xDiff = Math.abs(candAnchorVal - refXFromBbox(bbox));
            if (xDiff <= ANCHOR_MARGIN && xDiff < prevDiff) {
              candidateAnchor = candAnchorVal.toString();
              prevDiff = xDiff;
            }
          }
        }

        // No candidate found
        if (candidateAnchor.length == 0) {
          continue;
        }

        // Candidate found - update the anchor's bbox list
        // But first check if the bbox is already in this anchor (could happen after merging)
        const targetAnchor = collection[parseFloat(candidateAnchor)];
        if (!targetAnchor.includes(bbox)) {
          targetAnchor.push(bbox);
        }
      }
    }
  }

  // Try to left-align floating bboxes
  tryAlignFloating(
    anchorLeft,
    2,
    (bbox) => bbox.x,
    (bbox) => roundAnchor(bbox.x)
  );

  // Sort the anchors' lists of bboxes by y-value
  function sortAnchor(collection: Anchor) {
    for (const anchor in collection) {
      collection[anchor].sort((a, b) => a.y - b.y);
    }
  }

  sortAnchor(anchorLeft);
  sortAnchor(anchorRight);
  sortAnchor(anchorCenter);

  // deduplicate
  const duplicates = [];
  for (const anchor in anchorLeft) {
    for (const item of anchorLeft[anchor]) {
      item.snap = "left";
      item.leftAnchor = anchor;
    }
  }

  for (const anchor in anchorRight) {
    for (const item of anchorRight[anchor]) {
      if (item.snap) {
        item.isDup = true;
        duplicates.push(item);
      }
      item.snap = "right";
      item.rightAnchor = anchor;
    }
  }

  for (const anchor in anchorCenter) {
    for (const item of anchorCenter[anchor]) {
      if (item.snap && !item.isDup) {
        item.isDup = true;
        duplicates.push(item);
      }
      item.snap = "center";
      item.centerAnchor = anchor;
    }
  }

  function anchorCounts(item: ProjectionTextBox): number[] {
    let leftCount = 0;
    if (item.leftAnchor) {
      const key = parseFloat(item.leftAnchor);
      leftCount = anchorLeft[key] ? anchorLeft[key].length : 0;
    }
    let rightCount = 0;
    if (item.rightAnchor) {
      const key = parseFloat(item.rightAnchor);
      rightCount = anchorRight[key] ? anchorRight[key].length : 0;
    }
    let centerCount = 0;
    if (item.centerAnchor) {
      const key = parseFloat(item.centerAnchor);
      centerCount = anchorCenter[key] ? anchorCenter[key].length : 0;
    }
    return [leftCount, rightCount, centerCount];
  }

  // find all left aligned blocks, all right aligned blocks, all centered blocks, in that order
  // we cannot check all 3 at once since we may end up double counting potential anchor matches
  // (i.e. we need to exclude block that we know are left/right aligned before counting possible
  // matching centered blocks)

  // find all lefts
  let hasChanged = true;
  while (hasChanged && duplicates.length > 0) {
    hasChanged = false;
    for (let i = duplicates.length - 1; i >= 0; --i) {
      const item = duplicates[i];
      const [leftCount, rightCount, centerCount] = anchorCounts(item);
      if (leftCount >= rightCount && leftCount >= centerCount) {
        item.snap = "left";
        if (item.rightAnchor) {
          const key = parseFloat(item.rightAnchor);
          if (anchorRight[key]) {
            const idx = anchorRight[key].indexOf(item);
            if (idx >= 0) {
              anchorRight[key].splice(idx, 1);
              hasChanged = true;
            }
          }
        }
        if (item.centerAnchor) {
          const key = parseFloat(item.centerAnchor);
          if (anchorCenter[key]) {
            const idx = anchorCenter[key].indexOf(item);
            if (idx >= 0) {
              anchorCenter[key].splice(idx, 1);
              hasChanged = true;
            }
          }
        }
        duplicates.splice(i, 1);
      }
    }
  }

  // find all rights
  hasChanged = true;
  while (hasChanged && duplicates.length > 0) {
    hasChanged = false;
    for (let i = duplicates.length - 1; i >= 0; --i) {
      const item = duplicates[i];
      const [leftCount, rightCount, centerCount] = anchorCounts(item);
      if (rightCount >= leftCount && rightCount >= centerCount) {
        item.snap = "right";
        if (item.leftAnchor) {
          const key = parseFloat(item.leftAnchor);
          if (anchorLeft[key]) {
            const idx = anchorLeft[key].indexOf(item);
            if (idx >= 0) {
              anchorLeft[key].splice(idx, 1);
              hasChanged = true;
            }
          }
        }
        if (item.centerAnchor) {
          const key = parseFloat(item.centerAnchor);
          if (anchorCenter[key]) {
            const idx = anchorCenter[key].indexOf(item);
            if (idx >= 0) {
              anchorCenter[key].splice(idx, 1);
              hasChanged = true;
            }
          }
        }
        duplicates.splice(i, 1);
      }
    }
  }

  // remaining duplicates are centered
  for (const item of duplicates) {
    item.snap = "center";
    if (item.leftAnchor) {
      const key = parseFloat(item.leftAnchor);
      if (anchorLeft[key]) {
        const idx = anchorLeft[key].indexOf(item);
        if (idx >= 0) {
          anchorLeft[key].splice(idx, 1);
        }
      }
    }
    if (item.rightAnchor) {
      const key = parseFloat(item.rightAnchor);
      if (anchorRight[key]) {
        const idx = anchorRight[key].indexOf(item);
        if (idx >= 0) {
          anchorRight[key].splice(idx, 1);
        }
      }
    }
  }

  // filter anchors
  // delete singleton
  for (const anchor in anchorLeft) {
    if (anchorLeft[anchor].length < 2) {
      if (anchorLeft[anchor].length) {
        delete anchorLeft[anchor][0].snap;
      }
      delete anchorLeft[anchor];
    }
  }

  for (const anchor in anchorRight) {
    if (anchorRight[anchor].length < 2) {
      if (anchorRight[anchor].length) {
        delete anchorRight[anchor][0].snap;
      }
      delete anchorRight[anchor];
    }
  }

  for (const anchor in anchorCenter) {
    if (anchorCenter[anchor].length < 2) {
      if (anchorCenter[anchor].length) {
        delete anchorCenter[anchor][0].snap;
      }
      delete anchorCenter[anchor];
    }
  }

  return {
    anchorLeft,
    anchorRight,
    anchorCenter,
  };
}

function handleRotationReadingOrder(textBbox: ProjectionTextBox[], pageHeight: number) {
  // if no bbox is rotated (.r is set), return
  if (!textBbox.find((b) => b.r != 0)) {
    return;
  }

  // Group ALL items by rotation value (not by consecutive items)
  // This ensures rotated text blocks stay together even when their X coordinates
  // overlap with non-rotated content (e.g., rotated table + footer at same X positions)
  const groupsByRotation: { [key: number]: ProjectionTextBox[] } = {};
  for (const bbox of textBbox) {
    const r = bbox.r || 0;
    if (!groupsByRotation[r]) {
      groupsByRotation[r] = [];
    }
    groupsByRotation[r].push(bbox);
  }

  // Build bboxGroup array from rotation groups, sorted by X position of group
  const bboxGroup: ProjectionTextBox[][] = [];
  for (const rotation in groupsByRotation) {
    const group = groupsByRotation[rotation];
    // Sort each group by Y for proper reading order
    group.sort((a, b) => a.y - b.y);
    bboxGroup.push(group);
  }

  // Sort groups by their minimum X position to maintain left-to-right order
  bboxGroup.sort((a, b) => {
    const minXA = Math.min(...a.map((item) => item.x));
    const minXB = Math.min(...b.map((item) => item.x));
    return minXA - minXB;
  });

  // NOTE/ WARNING: height and width of bbox are NOT rotated beforehand!
  for (const [index, group] of bboxGroup.entries()) {
    if (group[0].r == 90 || group[0].r == 270) {
      // Check if there are non-rotated items that actually overlap visually (both X and Y)
      // with the rotated group. X-only overlap is not sufficient because items could
      // be in completely different parts of the page (e.g., rotated table + footer).
      let globalOverlap = false;
      for (const bbox of textBbox) {
        if (bbox.r != group[0].r) {
          const overlap = group.find(
            (b) =>
              // Check X overlap
              b.x >= bbox.x &&
              b.x <= bbox.x + bbox.w &&
              // Also check Y overlap - items must actually be near each other vertically
              b.y < bbox.y + bbox.h &&
              b.y + b.h > bbox.y &&
              bbox.r != b.r
          );
          if (overlap) {
            globalOverlap = true;
          }
        }
      }

      if (globalOverlap) {
        // rotate bbox to be horizontal
        for (const bbox of group) {
          if (bbox.d) {
            bbox.y += bbox.d;
            bbox.d = 0;
          }
          bbox.r = 0;
          bbox.rotated = true;
        }
      } else {
        // insert the bbox group in the Y axis after previous group and before next group.
        // move Next group by current group height (width as not rotated yet).
        const groupMaxX = Math.max(...group.map((v) => v.x + v.w));

        let deltaY = 0;
        if (index != 0) {
          const previousGroup = bboxGroup[index - 1];
          const previousGroupMaxY = Math.max(...previousGroup.map((v) => v.y + v.h));
          // Use pageHeight offset to guarantee no alignment issues with other groups
          deltaY = previousGroupMaxY + pageHeight;
        }
        // clockwise rotation (90 degrees)
        // - Text reads top-to-bottom in page space
        // - Y position in page space -> X position after de-rotation
        // - X position in page space -> Y position after de-rotation (row)
        if (group[0].r == 90) {
          for (const bbox of group) {
            const newX = Math.round(bbox.y);
            const newY = bbox.x + deltaY;
            // Swap width and height since text orientation changes
            const newW = bbox.h;
            const newH = bbox.w;
            bbox.x = newX;
            bbox.y = newY;
            bbox.w = newW;
            bbox.h = newH;
            bbox.r = 0;
            bbox.rotated = true;
          }
        }
        // counter clockwize rotation (text reads bottom-to-top)
        // For 270-degree rotation, text at higher Y positions should be
        // at lower X positions after de-rotation (left-to-right reading order)
        if (group[0].r == 270) {
          // For 270-degree counter-clockwise rotation:
          // - Text reads bottom-to-top in page space
          // - Y position in page space -> X position after de-rotation (inverted)
          // - X position in page space -> Y position after de-rotation (row)
          // - w and h need to be swapped since they represent visual dimensions
          // For 270-degree rotation: h is the extent along reading direction (string width)
          const maxY = Math.max(...group.map((b) => b.y + b.h));

          for (const bbox of group) {
            // Transform coordinates:
            // - new_x = distance from right edge of rotated block (inverted Y)
            //   Use h (string width in original coords) for the extent
            // - new_y = row position (from original X)
            const newX = Math.round(maxY - bbox.y - bbox.h);
            // Use exact X for Y (will be grouped by bboxToLine's Y_SORT_TOLERANCE)
            const newY = bbox.x + deltaY;
            // Swap width and height since text orientation changes
            const newW = bbox.h;
            const newH = bbox.w;
            bbox.x = newX;
            bbox.y = newY;
            bbox.w = newW;
            bbox.h = newH;
            bbox.r = 0;
            bbox.rotated = true;
          }
        }

        // Use pageHeight offset to guarantee no alignment issues
        const globalDelta = deltaY + groupMaxX + pageHeight;

        for (const [otherGroupIndex, other] of bboxGroup.entries()) {
          if (otherGroupIndex <= index) {
            continue;
          }
          for (const bbox of other) {
            if (bbox.r == 90 || bbox.r == 270) {
              bbox.d = (bbox.d ? bbox.d : 0) + globalDelta;
              continue;
            }
            bbox.y += globalDelta;
          }
        }
      }
    }
  }

  textBbox.sort((a, b) => {
    return a.y - b.y;
  });

  // Handle 180-degree rotated text (upside down)
  // Since we already grouped by rotation, we can iterate the existing groups
  for (const group of bboxGroup) {
    if (group[0].r == 180) {
      // Sort by X for proper reading order
      group.sort((a, b) => a.x - b.x);
      // Switch upside down
      for (const bbox of group) {
        bbox.x = Math.round(bbox.ry ?? bbox.y);
        bbox.y = bbox.rx ?? bbox.x;
        bbox.r = 0;
        bbox.rotated = true;
      }
    }
  }
}

export function bboxToLine(
  textBbox: ProjectionTextBox[],
  medianWidth: number,
  medianHeight: number,
  pageWidth?: number
): ProjectionTextBox[][] {
  // Y-tolerance for sorting: items within this threshold are considered same line
  // This handles:
  // 1. Floating point precision issues between columns (e.g., 334.7400 vs 334.7399)
  // 2. Subscripts/superscripts which are typically offset by 3-5 units from their base characters
  // Using a fraction of medianHeight to scale with document font size.
  const Y_SORT_TOLERANCE = Math.max(medianHeight * 0.5, 5.0);

  // Note: We keep whitespace items as they may be needed for proper word separation.
  // The spacing calculation handles gaps between items.

  // For two-column documents, detect and mark margin line numbers
  // These are short numeric items positioned between columns (near the page midpoint)
  // They should not be merged with column content
  if (pageWidth) {
    const midpoint = pageWidth * 0.5;
    const marginZoneLeft = midpoint - 5;
    const marginZoneRight = midpoint + 20;
    for (const bbox of textBbox) {
      const bboxCenter = bbox.x + bbox.w / 2;
      // Check if item is in the margin zone and looks like a line number
      if (
        bboxCenter > marginZoneLeft &&
        bboxCenter < marginZoneRight &&
        bbox.str.trim().match(/^\d{1,2}[O]?$/) && // 1-2 digits, possibly with O (OCR error for 0)
        bbox.w < 15 // Line numbers are narrow
      ) {
        // Mark as margin item - will be placed on its own line
        bbox.isMarginLineNumber = true;
      }
    }
  }

  // sort lines on first y axis then x axis (top - left)
  // Use Y tolerance so items on same visual line sort by x regardless of tiny y differences
  textBbox.sort((a, b) => {
    if (Math.abs(a.y - b.y) < Y_SORT_TOLERANCE) {
      return a.x - b.x;
    }
    return a.y - b.y;
  });

  function canMergeMarkup(previousBbox: ProjectionTextBox, bbox: ProjectionTextBox): boolean {
    if (!previousBbox.markup && !bbox.markup) {
      return true;
    }
    if (
      previousBbox.markup &&
      bbox.markup &&
      previousBbox.markup.highlight === bbox.markup.highlight &&
      previousBbox.markup.underline === bbox.markup.underline &&
      previousBbox.markup.squiggly === bbox.markup.squiggly &&
      previousBbox.markup.strikeout === bbox.markup.strikeout
    ) {
      return true;
    }
    return false;
  }

  function canMerge(previousBbox: ProjectionTextBox, bbox: ProjectionTextBox): boolean {
    if (bbox.y == previousBbox.y && bbox.h == previousBbox.h) {
      // Use raw pageBbox width for sub-pixel accurate gap calculation.
      // The rounded `.w` field can cause a true −0.02px overlap to appear as +0.12px,
      // and a −0.86px overlap to appear as −0.72px — both outside the old tolerance.
      // PDFs sometimes encode a single value (e.g. "119:12") as a sequence of adjacent
      // text runs whose bounding boxes touch or slightly overlap (up to ~1px) due to
      // character spacing / kerning. We must merge these rather than treat them as
      // separate tokens.
      const prevRawWidth = previousBbox.pageBbox?.w ?? previousBbox.w;
      const xDelta = bbox.x - previousBbox.x - prevRawWidth;
      if (
        ((xDelta < 0 && xDelta > -1.0) || (xDelta >= 0 && xDelta < 0.1)) &&
        canMergeMarkup(previousBbox, bbox)
      ) {
        return true;
      }
    }
    return false;
  }

  function mergePageBbox(a: ProjectionTextBox, b: ProjectionTextBox): Coordinates {
    const aBbox = a.pageBbox || { x: a.x, y: a.y, w: a.w, h: a.h };
    const bBbox = b.pageBbox || { x: b.x, y: b.y, w: b.w, h: b.h };
    const left = Math.min(aBbox.x, bBbox.x);
    const top = Math.min(aBbox.y, bBbox.y);
    const right = Math.max(aBbox.x + aBbox.w, bBbox.x + bBbox.w);
    const bottom = Math.max(aBbox.y + aBbox.h, bBbox.y + bBbox.h);
    return { x: left, y: top, w: right - left, h: bottom - top };
  }

  // merge Continuous bbox
  for (let i = 1; i < textBbox.length; i++) {
    const bbox = textBbox[i];
    const previousBbox = textBbox[i - 1];
    if (canMerge(previousBbox, bbox)) {
      previousBbox.w = bbox.x + bbox.w - previousBbox.x;
      previousBbox.str += bbox.str;
      previousBbox.strLength += bbox.strLength;
      previousBbox.pageBbox = mergePageBbox(previousBbox, bbox);
      textBbox.splice(i, 1);
      i--;
    }
  }

  // try to find the bounding box that align as line and group them by line
  const lines: ProjectionTextBox[][] = [];
  let currentLine: ProjectionTextBox[] = [];
  let previousBbox = null;

  for (const bbox of textBbox) {
    if (!previousBbox) {
      currentLine.push(bbox);
    }
    // This is where we define how line are build. to be improved
    else {
      const lineMinY = Math.min(...currentLine.map((v) => v.y));
      const lineMaxY = Math.max(...currentLine.map((v) => v.y + v.h));

      let lineCollide = false;
      for (const currentLineItemBbox of currentLine) {
        const overlapLenght =
          Math.min(currentLineItemBbox.x + currentLineItemBbox.w, bbox.x + bbox.w) -
          Math.max(currentLineItemBbox.x, bbox.x);
        // Use a minimum threshold to tolerate small overlaps common in PDFs due to:
        // - character spacing/kerning
        // - floating-point precision issues
        // - adjacent items with slightly overlapping bounding boxes
        // We want to detect true collisions (same text rendered twice) not adjacent text
        if (overlapLenght > Math.max(medianWidth / 3, 5)) {
          lineCollide = true;
          break;
        }
      }

      // Don't merge margin line numbers with regular content
      const currentLineHasMargin = currentLine.some((b) => b.isMarginLineNumber === true);
      const bboxIsMargin = bbox.isMarginLineNumber === true;
      const marginMismatch = currentLineHasMargin !== bboxIsMargin;

      // For rotated text, use Y-tolerance based merging since heights may be inconsistent
      const yTolerance = bbox.rotated ? Math.max(medianHeight * 2, 20) : 0;
      const yWithinTolerance = bbox.rotated && Math.abs(bbox.y - lineMinY) < yTolerance;

      if (
        !lineCollide &&
        !marginMismatch &&
        (yWithinTolerance ||
          (bbox.y + bbox.h * 0.5 >= lineMinY && bbox.y + bbox.h * 0.5 <= lineMaxY) ||
          (bbox.y >= lineMinY && bbox.y <= lineMaxY))
      ) {
        currentLine.push(bbox);
      } else {
        if (currentLine.length) {
          lines.push(currentLine);
        }
        currentLine = [bbox];
      }
    }
    previousBbox = bbox;
  }

  if (currentLine.length) {
    lines.push(currentLine);
  }

  // sort each line by x
  for (const line of lines) {
    line.sort((a, b) => a.x - b.x);
  }

  // sort lines by y
  lines.sort((a, b) => a[0].y - b[0].y);

  // merge 'words'
  const mergeThreshold = 1;

  // Pattern to detect standalone numeric values (financial table numbers)
  // Matches: numbers with optional commas, decimal points, dollar signs, percentages, negatives
  const numericPattern = /^[$]?-?[\d,]+\.?\d*%?$/;

  function looksLikeTableNumber(str: string): boolean {
    const trimmed = str.trim();
    // Must be at least 2 chars to be a table number (avoid merging single digits)
    return trimmed.length >= 2 && numericPattern.test(trimmed);
  }

  for (const line of lines) {
    for (let i = 1; i < line.length; ++i) {
      // merge box in word if:
      // - same height
      // - less than 2 in space
      // if (line[i].h == line[i-1].h) {
      const currentLine = line[i];
      const previousLine = line[i - 1];
      if (canMergeMarkup(previousLine, currentLine)) {
        // Don't merge adjacent numbers in tables - they're separate columns
        const bothAreNumbers =
          looksLikeTableNumber(previousLine.str) && looksLikeTableNumber(currentLine.str);

        // Check gap with BOTH rounded width (used elsewhere) and raw width from pageBbox.
        // Only merge without space if both agree the gap is small enough. This prevents
        // rounding artifacts from causing word fusions (e.g., "of" + "our" → "ofour"
        // when Math.round(w) reduces the gap from 1.34 to 1.0)
        const roundedGap = currentLine.x - previousLine.x - previousLine.w;
        const rawGap =
          currentLine.x - previousLine.x - (previousLine.pageBbox?.w ?? previousLine.w);
        if (!bothAreNumbers && roundedGap <= mergeThreshold && rawGap <= mergeThreshold) {
          // if same word but less than .7 of prev line
          if (currentLine.h != 0 && currentLine.h < previousLine.h * 0.7) {
            // and not starting with space
            if (currentLine.str[0] == " ") {
              break;
            }
            if (currentLine.y > previousLine.y + previousLine.h * 0.2) {
              currentLine.str = strToSubscriptString(currentLine.str);
            } else {
              currentLine.str = strToPostScript(currentLine.str);
            }
          }
          previousLine.w = currentLine.x + currentLine.w - previousLine.x;
          previousLine.str += currentLine.str;
          previousLine.strLength += currentLine.strLength;
          previousLine.pageBbox = mergePageBbox(previousLine, currentLine);
          line.splice(i, 1);
          i--;
        } else if (
          !bothAreNumbers &&
          currentLine.x - previousLine.x - previousLine.w < previousLine.w / previousLine.strLength
        ) {
          // merge if space between this word and previous is less than average
          // character width (using previous word font size)
          // But don't merge adjacent numbers - they're likely table columns

          // Now extend the width
          previousLine.w = currentLine.x + currentLine.w - previousLine.x;

          // Add space between merged items unless the previous already ends with space
          if (!previousLine.str.endsWith(" ")) {
            previousLine.str += " ";
            previousLine.strLength += 1;
          }
          previousLine.str += currentLine.str;
          previousLine.strLength += currentLine.strLength;
          previousLine.pageBbox = mergePageBbox(previousLine, currentLine);
          line.splice(i, 1);
          i--;
        }
      }
      // }
    }
  }

  // check if we can merge the lines together
  for (let i = 1; i < lines.length - 1; i++) {
    const currentLine = lines[i];
    const previousLine = lines[i - 1];

    const previousLineMinY = Math.min(...previousLine.map((v) => v.y));
    const previousLineMaxY = Math.max(...previousLine.map((v) => v.y + v.h));
    const currentLineMinY = Math.min(...currentLine.map((v) => v.y));
    const currentLineMaxY = Math.max(...currentLine.map((v) => v.y + v.h));

    // does the 2 line overlap?
    if (previousLineMaxY > currentLineMinY && previousLineMinY < currentLineMaxY) {
      // check the bboxes of current line and prevline do not overlap
      let bboxOverlap = false;
      for (const bbox of currentLine) {
        for (const prevBbox of previousLine) {
          if (bbox.x >= prevBbox.x && bbox.x <= prevBbox.x + prevBbox.w) {
            bboxOverlap = true;
            break;
          }
          if (prevBbox.x >= bbox.x && prevBbox.x <= bbox.x + bbox.w) {
            bboxOverlap = true;
            break;
          }
        }
      }

      // merge if no overlap
      if (!bboxOverlap) {
        previousLine.push(...currentLine);
        previousLine.sort((a, b) => a.x - b.x);
        lines.splice(i--, 1);
      }
    }
  }

  for (let i = 1; i < lines.length; i++) {
    const previousLineMetrics = getRepresentativeLineMetrics(lines[i - 1], medianHeight);
    const currentLineMetrics = getRepresentativeLineMetrics(lines[i], medianHeight);
    const yDelta = currentLineMetrics.top - previousLineMetrics.bottom;
    const referenceHeight = Math.max(
      medianHeight,
      Math.min(previousLineMetrics.height, currentLineMetrics.height)
    );

    // Calculate the number of blank lines to insert based on vertical spacing
    // Use medianHeight as a reference for one line spacing
    if (yDelta > referenceHeight) {
      // Calculate how many blank lines should be inserted
      // Round to nearest integer to get approximate number of lines
      const numBlankLines = Math.round(yDelta / referenceHeight) - 1;
      // Cap at a reasonable maximum (e.g., 10 blank lines) to avoid extreme cases
      const linesToInsert = Math.min(Math.max(numBlankLines, 1), 10);

      // Insert the calculated number of blank lines
      const blankLines = Array(linesToInsert).fill([]);
      lines.splice(i, 0, ...blankLines);
      i += linesToInsert;
    }
  }
  return lines;
}

function canRenderBbox(line: ProjectionTextBox[], bbox: ProjectionTextBox): boolean {
  for (const item of line) {
    if (item == bbox) {
      return true;
    }
    if (!item.rendered) {
      return false;
    }
  }
  return false;
}

function updateForwardAnchorRightBound(
  snapMap: number[],
  forwardAnchor: ForwardAnchor,
  rightBound: number,
  anchorTarget: number
): void {
  // Anything snapped to the right of rightBound should be aligned to anchorTarget line length at minimum
  // Also update nearby positions (within tolerance) to handle slight position variations between rows
  const POSITION_TOLERANCE = 2;

  for (let i = snapMap.length - 1; i >= 0; --i) {
    const anchor = snapMap[i];
    if (rightBound <= anchor) {
      if (!forwardAnchor[anchor] || anchorTarget > forwardAnchor[anchor]) {
        forwardAnchor[anchor] = anchorTarget;
      }
      // Also update nearby positions within tolerance
      for (let j = i - 1; j >= 0; --j) {
        const nearbyAnchor = snapMap[j];
        if (anchor - nearbyAnchor > POSITION_TOLERANCE) break;
        if (!forwardAnchor[nearbyAnchor] || anchorTarget > forwardAnchor[nearbyAnchor]) {
          forwardAnchor[nearbyAnchor] = anchorTarget;
        }
      }
    } else {
      return;
    }
  }
}

function updateForwardAnchors(
  bbox: ProjectionTextBox,
  nextBbox: ProjectionTextBox | null,
  snapMaps: SnapMaps,
  forwardAnchors: PageForwardAnchors,
  lineLength: number
): void {
  const rightBound = bbox.x + bbox.w;
  let targetLength = lineLength;
  if (nextBbox && (nextBbox.shouldSpace ?? 0) > 0) {
    targetLength += nextBbox.shouldSpace ?? 0;
  }
  updateForwardAnchorRightBound(snapMaps.left, forwardAnchors.left, rightBound, targetLength);
  updateForwardAnchorRightBound(snapMaps.right, forwardAnchors.right, rightBound, targetLength);
  // we do not update center anchors since centered text may span between snapped columns
  updateForwardAnchorRightBound(
    snapMaps.floating,
    forwardAnchors.floating,
    rightBound,
    targetLength
  );
}

/**
 * Compute the maximum gap between adjacent items on a line.
 */
function lineMaxGap(line: ProjectionTextBox[]): number {
  let maxGap = 0;
  for (let gi = 1; gi < line.length; gi++) {
    const gap = line[gi].x - (line[gi - 1].x + line[gi - 1].w);
    if (gap > maxGap) maxGap = gap;
  }
  return maxGap;
}

/**
 * Render a single line as flowing text: join items with single spaces based on gap size.
 * Sets bbox.rendered = true for each item.
 */
function renderLineAsFlowingText(
  line: ProjectionTextBox[],
  minX: number,
  medianWidth: number
): string {
  const indent = Math.min(
    Math.max(Math.round((line[0].x - minX) / medianWidth), 0),
    FLOWING_MAX_INDENT
  );
  let result = " ".repeat(indent);

  for (let i = 0; i < line.length; i++) {
    const bbox = line[i];
    if (i > 0) {
      const prevBbox = line[i - 1];
      const gap = bbox.x - (prevBbox.x + prevBbox.w);
      const spaceThreshold = Math.max(
        bbox.h * FLOWING_SPACE_HEIGHT_RATIO,
        FLOWING_SPACE_MIN_THRESHOLD
      );
      if (gap > spaceThreshold && !result.endsWith(" ")) {
        result += " ";
      }
    }
    result += bbox.str;
    bbox.rendered = true;
  }

  return result;
}

/**
 * Classify whether a block of lines is flowing paragraph text or structured/tabular content.
 * Flowing text gets a simpler rendering path that avoids grid projection artifacts.
 */
function isFlowingTextBlock(
  blockLines: ProjectionTextBox[][],
  anchorLeft: Anchor,
  anchorRight: Anchor,
  anchorCenter: Anchor,
  pageWidth: number
): boolean {
  const leftAnchorCount = Object.keys(anchorLeft).length;
  const rightAnchorCount = Object.keys(anchorRight).length;
  const centerAnchorCount = Object.keys(anchorCenter).length;

  // Multiple column anchors indicate structured/tabular content
  if (leftAnchorCount + rightAnchorCount + centerAnchorCount > FLOWING_MAX_TOTAL_ANCHORS)
    return false;
  if (leftAnchorCount > FLOWING_MAX_LEFT_ANCHORS) return false;

  // Count non-empty lines and how many span most of the page width
  let nonEmptyLines = 0;
  let wideLines = 0;
  for (const line of blockLines) {
    if (line.length === 0) continue;
    nonEmptyLines++;
    const lineStart = line[0].x;
    const lineEnd = line[line.length - 1].x + line[line.length - 1].w;
    if (lineEnd - lineStart > pageWidth * FLOWING_WIDE_LINE_RATIO) wideLines++;
  }

  // Need enough lines to confidently classify
  if (nonEmptyLines < FLOWING_MIN_LINES) return false;

  // Majority of lines should span most of page width for flowing text
  return wideLines / nonEmptyLines > FLOWING_WIDE_LINE_THRESHOLD;
}

/**
 * Render a flowing text block by joining items with single spaces.
 * Avoids grid projection artifacts (excessive whitespace, fused words)
 * that occur when applying column-alignment logic to paragraph text.
 */
function renderFlowingBlock(
  lines: ProjectionTextBox[][],
  block: LineRange,
  rawLines: string[],
  medianWidth: number
): void {
  // Find the block's left margin
  let minX = Infinity;
  for (let i = block.start; i < block.end; i++) {
    if (lines[i].length > 0) {
      minX = Math.min(minX, lines[i][0].x);
    }
  }
  if (minX === Infinity) minX = 0;

  for (let lineIndex = block.start; lineIndex < block.end; lineIndex++) {
    const line = lines[lineIndex];
    if (!rawLines[lineIndex]) {
      rawLines[lineIndex] = "";
    }
    if (line.length === 0) continue;

    rawLines[lineIndex] = renderLineAsFlowingText(line, minX, medianWidth);
  }
}

function getMedianTextBoxSize(lines: ProjectionTextBox[]): TextBoxSize {
  // calculate median textBox width
  const widthList = [];
  for (const bbox of lines) {
    if (bbox.w > 0) {
      widthList.push(bbox.w / bbox.strLength);
    }
  }
  const medianWidth = widthList.sort((a, b) => a - b)[Math.floor(widthList.length / 2)];

  // calculate median textBox height
  const heightList = [];
  for (const bbox of lines) {
    if (bbox.h > 0) {
      heightList.push(bbox.h);
    }
  }
  const medianHeight = heightList.sort((a, b) => a - b)[Math.floor(heightList.length / 2)];
  return { width: medianWidth, height: medianHeight };
}

export function projectToGrid(
  config: LiteParseConfig,
  page: PageData,
  projectionBoxes: ProjectionTextBox[],
  prevAnchors: PrevAnchors,
  totalPages: number,
  logger: GridDebugLogger
): { text: string; prevAnchors: PrevAnchors } {
  // detect '.' garbage in the lines
  let dotCount = 0;
  for (const bbox of projectionBoxes) {
    // check if bbox.str contains only dots
    if (bbox.str.match(/^\.+$/)) {
      dotCount++;
    }
  }

  if (dotCount > 100 && dotCount > projectionBoxes.length * 0.05) {
    // remove all dots and splice them from lines
    const newLines = [];
    for (const bbox of projectionBoxes) {
      if (bbox.str.match(/^\.+$/)) {
        continue;
      }
      if (bbox.str.match(/^·+$/)) {
        continue;
      }
      if (bbox.str.match(/^"+$/)) {
        continue;
      }
      newLines.push(bbox);
    }
    projectionBoxes = newLines;
  }

  // calculate median textBox width/height
  const pageMedianSizes = getMedianTextBoxSize(projectionBoxes);
  let medianWidth = pageMedianSizes.width;
  const medianHeight = pageMedianSizes.height;

  // Save original bboxes (including OCR) for text attribution
  const attributionBboxes: ProjectionTextBox[] = [];
  for (const bbox of projectionBoxes as ProjectionTextBox[]) {
    if (!bbox || !bbox.str || bbox.vgap || bbox.isPlaceholder) {
      continue;
    }
    attributionBboxes.push({
      str: bbox.str,
      x: bbox.x,
      y: bbox.y,
      w: bbox.w,
      h: bbox.h,
      r: bbox.r,
      strLength: bbox.str.length,
    });
  }

  handleRotationReadingOrder(projectionBoxes, page.height);
  const lines = bboxToLine(projectionBoxes, medianWidth, medianHeight, page.width);

  // Log line composition
  for (let i = 0; i < lines.length; i++) {
    logger.logLineComposition(i, lines[i]);
  }

  // remove unprojectable text and apply markup to final lines
  for (let i = 0; i < lines.length; ++i) {
    const line = filterUnprojectableText(config, lines[i]);
    for (const bbox of line) {
      // With the way our grid projection currently works, we have to output
      // tags before raw line projection to avoid breaking the projection alignment.
      // The tags get replaced with MD as needed in output formatting, this does
      // result in output text containing the ~~ strikeout markup, but this is
      // mitigated since we skip markup entirely when we are not outputting markdown
      if (bbox.str.trim().length != 0 && bbox.markup) {
        bbox.str = applyMarkupTags(bbox.markup, bbox.str);
      }
    }
    lines[i] = line;
  }

  const forwardAnchors: PageForwardAnchors = {
    left: {},
    right: {},
    center: {},
    floating: {},
  };

  const rawLines: string[] = [];
  const rawLinesDelta = [];

  const blocks: LineRange[] = [];
  if (config.preserveLayoutAlignmentAcrossPages && totalPages > 1) {
    blocks.push({ start: 0, end: lines.length });
  } else {
    let emptyCount = 0;
    let start = -1;
    for (const [lineIndex, line] of lines.entries()) {
      if (line.length === 0) {
        emptyCount++;
        if (emptyCount > 1) {
          if (start >= 0) {
            // ignore completely empty blocks, include the double blank
            // line at the end of valid blocks
            blocks.push({ start: start, end: lineIndex + 1 });
          }
          start = -1;
        }
      } else {
        emptyCount = 0;
        if (start < 0) {
          start = lineIndex;
        }
      }
    }
    if (start > -1) {
      blocks.push({ start: start, end: lines.length });
    }
  }

  // Log block assignments
  for (let bi = 0; bi < blocks.length; bi++) {
    logger.logBlock(bi, blocks[bi].start, blocks[bi].end);
  }

  for (const block of blocks) {
    const blockLines = lines.slice(block.start, block.end);
    const { anchorLeft, anchorRight, anchorCenter } = extractAnchorsPointsFromLines(
      blockLines,
      page
    );

    logger.logAnchors(anchorLeft, anchorRight, anchorCenter);

    // Block-level classification: if entire block is clearly flowing text,
    // render it simply and skip grid projection
    if (isFlowingTextBlock(blockLines, anchorLeft, anchorRight, anchorCenter, page.width)) {
      logger.logFlowingBlock(block.start, block.end);
      if (!config.preserveLayoutAlignmentAcrossPages) {
        const sizes = getMedianTextBoxSize(blockLines.flat());
        medianWidth = sizes.width;
      }
      renderFlowingBlock(lines, block, rawLines, medianWidth);
      for (let li = block.start; li < block.end; li++) {
        if (rawLines[li]) logger.captureRender(li, 0, rawLines[li], "flowing");
      }
      continue;
    }

    logger.logStructuredBlock(block.start, block.end);

    const snapMaps: SnapMaps = {
      left: [],
      right: [],
      center: [],
      floating: [],
    };

    const uniqueSnaps = new Set<number>();
    for (const snap in anchorLeft) {
      uniqueSnaps.add(parseFloat(snap));
    }
    snapMaps.left.push(...uniqueSnaps);
    uniqueSnaps.clear();

    for (const snap in anchorRight) {
      uniqueSnaps.add(parseFloat(snap));
    }
    snapMaps.right.push(...uniqueSnaps);
    uniqueSnaps.clear();

    for (const snap in anchorCenter) {
      uniqueSnaps.add(parseFloat(snap));
    }
    snapMaps.center.push(...uniqueSnaps);
    uniqueSnaps.clear();

    let hasChanged = true;
    const leftSnap: Snap[] = [];
    const rightSnap: Snap[] = [];
    const centerSnap: Snap[] = [];

    if (!config.preserveLayoutAlignmentAcrossPages) {
      const sizes = getMedianTextBoxSize(lines.slice(block.start, block.end).flat());
      medianWidth = sizes.width;

      // medianHeight updated but not currently used per-block - reserved for future use
      void sizes.height;
    }

    // compute snaps
    for (let lineIndex = block.start; lineIndex < block.end; ++lineIndex) {
      const line = lines[lineIndex];
      const forceUnsnapped = !canSnapLine(config, line);
      let prevBbox = null;

      for (let boxIndex = 0; boxIndex < line.length; ++boxIndex) {
        const bbox = line[boxIndex];
        bbox.forceUnsnapped = forceUnsnapped;

        const spaceThreshold = 2;
        // should we add a space between the two bbox?
        // TODO RTL
        if (prevBbox && bbox.x - (prevBbox.x + prevBbox.w) > spaceThreshold) {
          const xDelta = bbox.x - (prevBbox.x + prevBbox.w);
          const prevCharWidth = prevBbox.w / prevBbox.strLength;

          // add a space
          bbox.shouldSpace = 1;

          if (xDelta > prevCharWidth * 2) {
            // Check if both items are in the same column based on gap size
            // If gap is less than 10% of page width, treat as same column
            // This works for any number of columns
            const columnGapThreshold = page.width * 0.1;
            const bothInSameColumn = xDelta < columnGapThreshold;

            // insert column spacing if any of:
            // - gap is more than an approximate tab (8x average char width)
            // - previous bbox is right snap
            // - this bbox is left snap
            // - both previous and this bbox are snaps
            // otherwise insert floating spacing
            if (
              (!bbox.forceUnsnapped && xDelta > prevCharWidth * 8) ||
              (bbox.snap && bbox.snap === "left") ||
              (prevBbox.snap && prevBbox.snap === "right") ||
              (bbox.snap && prevBbox.snap)
            ) {
              // If both items are in the same column, limit spacing to avoid
              // preserving justified text gaps from PDFs
              bbox.shouldSpace = bothInSameColumn ? FLOATING_SPACES : COLUMN_SPACES;
            } else {
              // For items in the same column, use minimal spacing
              bbox.shouldSpace = bothInSameColumn ? 1 : FLOATING_SPACES;
            }
          }
        } else {
          bbox.shouldSpace = 0;
        }

        prevBbox = bbox;
        logger.logSnapAssignment(bbox, lineIndex, boxIndex);
        if (!bbox.snap) {
          uniqueSnaps.add(Math.round(bbox.x));
        } else if (bbox.snap == "left") {
          leftSnap.push({ bbox, lineIndex, boxIndex });
        } else if (bbox.snap == "right") {
          rightSnap.push({ bbox, lineIndex, boxIndex });
        } else if (bbox.snap == "center") {
          centerSnap.push({ bbox, lineIndex, boxIndex });
        }
      }
    }

    snapMaps.floating.push(...uniqueSnaps);
    uniqueSnaps.clear();

    snapMaps.floating.sort((a, b) => a - b);
    snapMaps.center.sort((a, b) => a - b);
    snapMaps.right.sort((a, b) => a - b);
    snapMaps.left.sort((a, b) => a - b);

    // Per-line flowing text detection: pre-render lines that are clearly paragraph text
    // (spans page width, no large column gaps) with simple single-space joining.
    // This avoids grid projection artifacts on flowing text within mixed blocks.
    const flowingLines = new Set<number>();
    {
      // Find block's left margin for indent calculation
      let blockMinX = Infinity;
      for (let li = block.start; li < block.end; li++) {
        if (lines[li].length > 0) {
          blockMinX = Math.min(blockMinX, lines[li][0].x);
        }
      }
      if (blockMinX === Infinity) blockMinX = 0;

      const columnGapThreshold = medianWidth * FLOWING_COLUMN_GAP_MULTIPLIER;

      // Helper to mark and render a line as flowing
      function markFlowing(lineIndex: number, reason: string): void {
        const line = lines[lineIndex];
        if (!rawLines[lineIndex]) {
          rawLines[lineIndex] = "";
          rawLinesDelta[lineIndex] = 0;
        }
        rawLines[lineIndex] = renderLineAsFlowingText(line, blockMinX, medianWidth);
        flowingLines.add(lineIndex);
        logger.logFlowingLine(lineIndex, reason);
        logger.captureRender(lineIndex, 0, rawLines[lineIndex], "flowing");
      }

      // First pass: detect clearly flowing lines (wide, no column gaps, enough items)
      for (let lineIndex = block.start; lineIndex < block.end; lineIndex++) {
        const line = lines[lineIndex];
        if (line.length < FLOWING_MIN_LINE_ITEMS) continue;

        const lineStart = line[0].x;
        const lineEnd = line[line.length - 1].x + line[line.length - 1].w;
        const lineSpan = lineEnd - lineStart;

        if (
          lineSpan > page.width * FLOWING_WIDE_LINE_RATIO &&
          lineMaxGap(line) < columnGapThreshold
        ) {
          markFlowing(lineIndex, "wide span, no column gaps");
        }
      }

      // Second pass: extend flowing to adjacent continuation lines using
      // forward + backward sweeps (O(n) instead of iterating until convergence)
      // Forward sweep: propagate flowing status downward
      for (let lineIndex = block.start; lineIndex < block.end; lineIndex++) {
        if (flowingLines.has(lineIndex)) continue;
        const line = lines[lineIndex];
        if (line.length === 0) continue;
        if (flowingLines.has(lineIndex - 1) && lineMaxGap(line) < columnGapThreshold) {
          markFlowing(lineIndex, `forward propagation from line ${lineIndex - 1}`);
        }
      }
      // Backward sweep: propagate flowing status upward
      for (let lineIndex = block.end - 1; lineIndex >= block.start; lineIndex--) {
        if (flowingLines.has(lineIndex)) continue;
        const line = lines[lineIndex];
        if (line.length === 0) continue;
        if (flowingLines.has(lineIndex + 1) && lineMaxGap(line) < columnGapThreshold) {
          markFlowing(lineIndex, `backward propagation from line ${lineIndex + 1}`);
        }
      }
    }

    while (hasChanged || snapMaps.right.length || snapMaps.left.length || snapMaps.center.length) {
      hasChanged = false;

      for (let lineIndex = block.start; lineIndex < block.end; ++lineIndex) {
        const line = lines[lineIndex];
        if (!rawLines[lineIndex]) {
          rawLines[lineIndex] = "";
          rawLinesDelta[lineIndex] = 0;
        }

        for (let boxIndex = 0; boxIndex < line.length; ++boxIndex) {
          const bbox = line[boxIndex];
          if (bbox.rendered) {
            continue;
          }

          if (!bbox.forceUnsnapped) {
            if (bbox.snap) {
              continue;
            }

            if (
              (snapMaps.left.length && snapMaps.left[0] < bbox.x) ||
              (snapMaps.right.length && snapMaps.right[0] < bbox.x) ||
              (snapMaps.center.length && snapMaps.center[0] < Math.round(bbox.x + bbox.w / 2))
            ) {
              continue;
            }
          }

          if (!canRenderBbox(line, bbox)) {
            break;
          }

          let targetX = Math.min(Math.round(bbox.x / medianWidth), COLUMN_SPACES);

          let lastSnapLeft = 0;
          for (const key in forwardAnchors.left) {
            // Use parseFloat to preserve decimal precision from anchor keys
            if (parseFloat(key) <= bbox.x) {
              lastSnapLeft = Math.max(lastSnapLeft, forwardAnchors.left[key]);
            }
          }
          const lineMax = Math.max(
            lastSnapLeft,
            rawLines[lineIndex].trimEnd().length + (bbox.shouldSpace ?? 0)
          );
          if (targetX < lineMax) {
            targetX = lineMax;
          }

          if (!bbox.forceUnsnapped) {
            const floatingAnchor = forwardAnchors.floating[Math.round(bbox.x)];
            if (floatingAnchor && targetX < floatingAnchor) {
              // Limit floating anchor adjustment to avoid excessive gaps in justified text
              // Use a small max gap to prevent large spacing within columns
              const maxFloatingGap = 4;
              const adjustedAnchor = Math.min(floatingAnchor, targetX + maxFloatingGap);
              if (adjustedAnchor > targetX) {
                targetX = adjustedAnchor;
              }
            }
          }

          rawLines[lineIndex] = rawLines[lineIndex].trimEnd();
          if (targetX > rawLines[lineIndex].length) {
            rawLines[lineIndex] += " ".repeat(targetX - rawLines[lineIndex].length);
          }

          rawLines[lineIndex] += bbox.str;

          bbox.rendered = true;
          hasChanged = true;
          logger.logRender(bbox, lineIndex, targetX, "floating");
          logger.captureRender(lineIndex, targetX, bbox.str, "floating");

          let nextBbox: ProjectionTextBox | null = null;
          if (line.length > boxIndex + 1) {
            nextBbox = line[boxIndex + 1];
          }
          if (!bbox.forceUnsnapped) {
            updateForwardAnchors(
              bbox,
              nextBbox,
              snapMaps,
              forwardAnchors,
              rawLines[lineIndex].length
            );
          }
        }
      }

      if (
        snapMaps.left.length &&
        (!snapMaps.right.length || snapMaps.left[0] <= snapMaps.right[0]) &&
        (!snapMaps.center.length || snapMaps.left[0] <= snapMaps.center[0])
      ) {
        const thisTurnSnap: Snap[] = [];
        for (const item of leftSnap) {
          if (
            item.bbox.leftAnchor &&
            parseFloat(item.bbox.leftAnchor) == snapMaps.left[0] &&
            !flowingLines.has(item.lineIndex)
          ) {
            thisTurnSnap.push(item);
          }
        }
        hasChanged = true;
        if (!thisTurnSnap.length) {
          snapMaps.left.shift();
          continue;
        }

        let targetX = Math.min(Math.round(snapMaps.left[0] / medianWidth), COLUMN_SPACES);
        const lineMax = Math.max(
          ...thisTurnSnap.map((v) => {
            let spaceEnd = 0;
            if (!rawLines[v.lineIndex].endsWith(" ")) {
              spaceEnd = v.bbox.shouldSpace ?? 0;
            }
            if ((v.bbox.shouldSpace ?? 0) > 1) {
              const trailingSpaces =
                rawLines[v.lineIndex].length - rawLines[v.lineIndex].trimEnd().length;
              if (trailingSpaces < (v.bbox.shouldSpace ?? 0)) {
                spaceEnd = (v.bbox.shouldSpace ?? 0) - trailingSpaces;
              }
            }

            return rawLines[v.lineIndex].length + spaceEnd + 1;
          })
        );

        if (targetX < lineMax) {
          targetX = lineMax;
        }

        if (
          forwardAnchors.left[snapMaps.left[0]] &&
          targetX < forwardAnchors.left[snapMaps.left[0]]
        ) {
          targetX = forwardAnchors.left[snapMaps.left[0]];
        }
        if (
          prevAnchors.forwardAnchorLeft[snapMaps.left[0]] &&
          targetX < prevAnchors.forwardAnchorLeft[snapMaps.left[0]]
        ) {
          targetX = prevAnchors.forwardAnchorLeft[snapMaps.left[0]];
        }

        forwardAnchors.left[snapMaps.left[0]] = targetX;
        logger.logForwardAnchor("left", snapMaps.left[0], targetX);

        for (const currentLeftSnapBox of thisTurnSnap) {
          const lineIndex = currentLeftSnapBox.lineIndex;
          if (flowingLines.has(lineIndex)) continue;
          if (targetX > rawLines[lineIndex].length) {
            rawLines[lineIndex] += " ".repeat(targetX - rawLines[lineIndex].length);
          }
          rawLines[lineIndex] += currentLeftSnapBox.bbox.str;
          currentLeftSnapBox.bbox.rendered = true;
          logger.logRender(currentLeftSnapBox.bbox, lineIndex, targetX, "left-snap");
          logger.captureRender(lineIndex, targetX, currentLeftSnapBox.bbox.str, "left");

          let nextBbox: ProjectionTextBox | null = null;
          if (lines[lineIndex].length > currentLeftSnapBox.boxIndex + 1) {
            nextBbox = lines[lineIndex][currentLeftSnapBox.boxIndex + 1];
          }
          updateForwardAnchors(
            currentLeftSnapBox.bbox,
            nextBbox,
            snapMaps,
            forwardAnchors,
            rawLines[lineIndex].length
          );
        }

        for (let index = block.start; index < block.end; ++index) {
          if (flowingLines.has(index)) continue;
          const line = rawLines[index];
          if (line.length < targetX) {
            rawLines[index] += " ".repeat(targetX - line.length);
          }
        }
        snapMaps.left.shift();
      } else if (
        snapMaps.right.length &&
        (!snapMaps.left.length || snapMaps.right[0] <= snapMaps.left[0]) &&
        (!snapMaps.center.length || snapMaps.right[0] <= snapMaps.center[0])
      ) {
        const thisTurnSnap: Snap[] = [];
        hasChanged = true;
        for (const item of rightSnap) {
          if (
            item.bbox.rightAnchor &&
            parseFloat(item.bbox.rightAnchor) == snapMaps.right[0] &&
            !flowingLines.has(item.lineIndex)
          ) {
            thisTurnSnap.push(item);
          }
        }

        if (!thisTurnSnap.length) {
          snapMaps.right.shift();
          continue;
        }

        let targetX = Math.min(Math.round(snapMaps.right[0] / medianWidth), COLUMN_SPACES);
        const lineMax = Math.max(
          ...thisTurnSnap.map((v) => {
            let lastSnapLeft = 0;
            for (const key in forwardAnchors.left) {
              if (parseInt(key) <= v.bbox.x) {
                lastSnapLeft = Math.max(lastSnapLeft, forwardAnchors.left[key]);
              }
            }
            return (
              Math.max(
                lastSnapLeft,
                rawLines[v.lineIndex].trimEnd().length + (v.bbox.shouldSpace ?? 0)
              ) + v.bbox.strLength
            );
          })
        );

        if (targetX < lineMax) {
          targetX = lineMax;
        }
        if (
          forwardAnchors.right[snapMaps.right[0]] &&
          targetX < forwardAnchors.right[snapMaps.right[0]]
        ) {
          targetX = forwardAnchors.right[snapMaps.right[0]];
        }
        if (
          prevAnchors.forwardAnchorRight[snapMaps.right[0]] &&
          targetX < prevAnchors.forwardAnchorRight[snapMaps.right[0]]
        ) {
          targetX = prevAnchors.forwardAnchorRight[snapMaps.right[0]];
        }
        forwardAnchors.right[snapMaps.right[0]] = targetX;
        logger.logForwardAnchor("right", snapMaps.right[0], targetX);

        for (const currentRightSnapBox of thisTurnSnap) {
          const lineIndex = currentRightSnapBox.lineIndex;
          if (flowingLines.has(lineIndex)) continue;
          rawLines[lineIndex] = rawLines[lineIndex].trimEnd();
          if (targetX > rawLines[lineIndex].trimEnd().length + currentRightSnapBox.bbox.strLength) {
            rawLines[lineIndex] += " ".repeat(
              targetX - rawLines[lineIndex].length - currentRightSnapBox.bbox.strLength
            );
          }
          rawLines[lineIndex] += currentRightSnapBox.bbox.str;
          currentRightSnapBox.bbox.rendered = true;
          logger.logRender(currentRightSnapBox.bbox, lineIndex, targetX, "right-snap");
          logger.captureRender(
            lineIndex,
            targetX - currentRightSnapBox.bbox.strLength,
            currentRightSnapBox.bbox.str,
            "right"
          );

          let nextBbox: ProjectionTextBox | null = null;
          if (lines[lineIndex].length > currentRightSnapBox.boxIndex + 1) {
            nextBbox = lines[lineIndex][currentRightSnapBox.boxIndex + 1];
          }
          updateForwardAnchors(
            currentRightSnapBox.bbox,
            nextBbox,
            snapMaps,
            forwardAnchors,
            rawLines[lineIndex].length
          );
        }
        for (let index = block.start; index < block.end; ++index) {
          if (flowingLines.has(index)) continue;
          const line = rawLines[index];
          if (line.length < targetX) {
            rawLines[index] += " ".repeat(targetX - line.length);
          }
        }
        snapMaps.right.shift();
      } else if (
        snapMaps.center.length &&
        (!snapMaps.left.length || snapMaps.center[0] <= snapMaps.left[0]) &&
        (!snapMaps.right.length || snapMaps.center[0] <= snapMaps.right[0])
      ) {
        const thisTurnSnap: Snap[] = [];
        hasChanged = true;
        for (const item of centerSnap) {
          if (
            item.bbox.centerAnchor &&
            parseFloat(item.bbox.centerAnchor) == snapMaps.center[0] &&
            !flowingLines.has(item.lineIndex)
          ) {
            thisTurnSnap.push(item);
          }
        }
        if (!thisTurnSnap.length) {
          snapMaps.center.shift();
          continue;
        }
        let targetX = Math.min(Math.round(snapMaps.center[0] / medianWidth), COLUMN_SPACES);
        const lineMax = Math.max(
          ...thisTurnSnap.map((v) => {
            let spaceEnd = 0;
            if (!rawLines[v.lineIndex].endsWith(" ")) {
              spaceEnd = v.bbox.shouldSpace ?? 0;
            }
            if ((v.bbox.shouldSpace ?? 0) > 1) {
              const trailingSpaces =
                rawLines[v.lineIndex].length - rawLines[v.lineIndex].trimEnd().length;
              if (trailingSpaces < (v.bbox.shouldSpace ?? 0)) {
                spaceEnd = (v.bbox.shouldSpace ?? 0) - trailingSpaces;
              }
            }
            return rawLines[v.lineIndex].length + Math.round(v.bbox.strLength / 2) + spaceEnd;
          })
        );

        if (targetX < lineMax) {
          targetX = lineMax;
        }
        if (
          forwardAnchors.center[snapMaps.center[0]] &&
          targetX < forwardAnchors.center[snapMaps.center[0]]
        ) {
          targetX = forwardAnchors.center[snapMaps.center[0]];
        }
        if (
          prevAnchors.forwardAnchorCenter[snapMaps.center[0]] &&
          targetX < prevAnchors.forwardAnchorCenter[snapMaps.center[0]]
        ) {
          targetX = prevAnchors.forwardAnchorCenter[snapMaps.center[0]];
        }
        forwardAnchors.center[snapMaps.center[0]] = targetX;
        logger.logForwardAnchor("center", snapMaps.center[0], targetX);
        for (const currentCenterSnapBox of thisTurnSnap) {
          if (flowingLines.has(currentCenterSnapBox.lineIndex)) continue;
          if (
            targetX >
            rawLines[currentCenterSnapBox.lineIndex].length +
              Math.round(currentCenterSnapBox.bbox.strLength / 2)
          ) {
            rawLines[currentCenterSnapBox.lineIndex] += " ".repeat(
              targetX -
                rawLines[currentCenterSnapBox.lineIndex].length -
                Math.round(currentCenterSnapBox.bbox.strLength / 2)
            );
          }
          rawLines[currentCenterSnapBox.lineIndex] += currentCenterSnapBox.bbox.str;
          currentCenterSnapBox.bbox.rendered = true;
          logger.logRender(
            currentCenterSnapBox.bbox,
            currentCenterSnapBox.lineIndex,
            targetX,
            "center-snap"
          );
          logger.captureRender(
            currentCenterSnapBox.lineIndex,
            targetX - Math.round(currentCenterSnapBox.bbox.strLength / 2),
            currentCenterSnapBox.bbox.str,
            "center"
          );
        }
        snapMaps.center.shift();
      }
    }
  }

  fixSparseBlocks(blocks, rawLines);
  logger.captureRawLines(rawLines);

  const text = rawLines.join("\n");
  // OSS: Return text instead of mutating page object
  return {
    text,
    prevAnchors: {
      forwardAnchorLeft: forwardAnchors.left,
      forwardAnchorRight: forwardAnchors.right,
      forwardAnchorCenter: forwardAnchors.center,
    },
  };
}

export async function projectPagesToGrid(
  pages: PageData[],
  config: LiteParseConfig
): Promise<ParsedPage[]> {
  const logger = createGridDebugLogger(config.debug);
  const prevAnchors: PrevAnchors = {
    forwardAnchorLeft: {},
    forwardAnchorRight: {},
    forwardAnchorCenter: {},
  };

  const results: ParsedPage[] = [];

  for (const page of pages) {
    logger.setPage(page.pageNum, page.width, page.height);

    // Build projection boxes from text items
    const projectionBoxes = buildBbox(page, config);

    // Project to grid
    const { text, prevAnchors: newAnchors } = projectToGrid(
      config,
      page,
      projectionBoxes,
      prevAnchors,
      pages.length,
      logger
    );

    // Update forward anchors if preserving across pages
    if (config.preserveLayoutAlignmentAcrossPages) {
      for (const anchor in newAnchors.forwardAnchorLeft) {
        prevAnchors.forwardAnchorLeft[anchor] = newAnchors.forwardAnchorLeft[anchor];
      }
      for (const anchor in newAnchors.forwardAnchorRight) {
        prevAnchors.forwardAnchorRight[anchor] = newAnchors.forwardAnchorRight[anchor];
      }
      for (const anchor in newAnchors.forwardAnchorCenter) {
        prevAnchors.forwardAnchorCenter[anchor] = newAnchors.forwardAnchorCenter[anchor];
      }
    }

    // Build result page
    results.push({
      pageNum: page.pageNum,
      width: page.width,
      height: page.height,
      text,
      textItems: page.textItems,
      boundingBoxes: [],
    });
  }

  // Clean raw text (margin detection, etc)
  cleanRawText(results, config);

  // Flush debug log and render visualizations
  if (logger.enabled) {
    await logger.flush();
    if (logger.shouldVisualize) {
      const vizPath = logger.debugConfig.visualizePath ?? "./debug-output";
      const paths = await renderAllVisualizations(logger.visualizerPages, vizPath);
      for (const p of paths) {
        process.stderr.write(`[grid-debug] Visualization: ${p}\n`);
      }
    }
  }

  return results;
}
