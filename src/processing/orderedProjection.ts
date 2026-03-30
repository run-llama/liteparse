import { PageData } from "../engines/pdf/interface.js";
import { ParsedPage, LiteParseConfig, TextItem } from "../core/types.js";

/**
 * Union-Find (disjoint set) data structure for connected component detection.
 */
class UnionFind {
  private parent: number[];
  private rank: number[];

  constructor(n: number) {
    this.parent = Array.from({ length: n }, (_, i) => i);
    this.rank = new Array(n).fill(0);
  }

  find(x: number): number {
    if (this.parent[x] !== x) {
      this.parent[x] = this.find(this.parent[x]);
    }
    return this.parent[x];
  }

  union(a: number, b: number): void {
    const ra = this.find(a);
    const rb = this.find(b);
    if (ra === rb) return;

    if (this.rank[ra] < this.rank[rb]) {
      this.parent[ra] = rb;
    } else if (this.rank[ra] > this.rank[rb]) {
      this.parent[rb] = ra;
    } else {
      this.parent[rb] = ra;
      this.rank[ra]++;
    }
  }

  getComponents(): Map<number, number[]> {
    const components = new Map<number, number[]>();
    for (let i = 0; i < this.parent.length; i++) {
      const root = this.find(i);
      if (!components.has(root)) {
        components.set(root, []);
      }
      components.get(root)!.push(i);
    }
    return components;
  }
}

interface InflatedBox {
  index: number;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/**
 * Compute median of an array of numbers.
 */
function median(values: number[]): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[mid - 1] + sorted[mid]) / 2
    : sorted[mid];
}

/**
 * Check if two axis-aligned rectangles overlap.
 */
function rectsOverlap(a: InflatedBox, b: InflatedBox): boolean {
  return a.x1 < b.x2 && a.x2 > b.x1 && a.y1 < b.y2 && a.y2 > b.y1;
}

/**
 * Groups text items into content blobs using flood-fill (connected components
 * on inflated bounding boxes), then sorts blobs in reading order.
 *
 * Each text item's bbox is inflated by a factor of its own font size, so
 * nearby items of similar size merge into the same blob while distant
 * content (headers, sidebars, footnotes) stays separate.
 */
export function projectPagesOrdered(
  pages: PageData[],
  config: LiteParseConfig,
): ParsedPage[] {
  return pages.map((page) => {
    const text = projectPageOrdered(page, config);
    return {
      pageNum: page.pageNum,
      width: page.width,
      height: page.height,
      text,
      textItems: page.textItems,
    };
  });
}

function projectPageOrdered(
  page: PageData,
  _config: LiteParseConfig,
): string {
  const items = page.textItems.filter(
    (item) => item.str.trim().length > 0 && !item.isPlaceholder,
  );

  if (items.length === 0) return "";

  // Compute median font size as baseline unit
  const fontSizes = items
    .map((item) => item.fontSize ?? item.h)
    .filter((s) => s > 0);
  const medianFontSize = median(fontSizes);
  // Fallback if somehow all zero
  const unit = medianFontSize > 0 ? medianFontSize : 12;

  // Inflate each bbox. Use per-item font size for adaptive inflation.
  // Inflate more horizontally (text flows left-right) than vertically (lines are close).
  const inflated: InflatedBox[] = items.map((item, i) => {
    const fontSize = item.fontSize ?? item.h;
    const itemUnit = fontSize > 0 ? fontSize : unit;
    // Horizontal: inflate by 0.3x font size (bridge word gaps)
    // Vertical: inflate by 0.5x font size (bridge line gaps within a paragraph)
    const hPad = itemUnit * 0.3;
    const vPad = itemUnit * 0.5;
    return {
      index: i,
      x1: item.x - hPad,
      y1: item.y - vPad,
      x2: item.x + item.w + hPad,
      y2: item.y + item.h + vPad,
    };
  });

  // Find connected components using sweep-line + union-find.
  // Sort by y1 so we only need to check boxes within vertical range.
  const sortedByY = [...inflated].sort((a, b) => a.y1 - b.y1);

  const uf = new UnionFind(items.length);

  // For each box, check subsequent boxes that could overlap vertically.
  for (let i = 0; i < sortedByY.length; i++) {
    const a = sortedByY[i];
    for (let j = i + 1; j < sortedByY.length; j++) {
      const b = sortedByY[j];
      // Since sorted by y1, if b.y1 > a.y2 then no further boxes can overlap with a vertically.
      if (b.y1 > a.y2) break;
      if (rectsOverlap(a, b)) {
        uf.union(a.index, b.index);
      }
    }
  }

  // Collect components
  const components = uf.getComponents();

  // Build blobs: for each component, compute bounding rect and collect items
  const blobs: { minX: number; minY: number; items: TextItem[] }[] = [];

  for (const indices of components.values()) {
    let minX = Infinity;
    let minY = Infinity;
    const blobItems: TextItem[] = [];
    for (const idx of indices) {
      const item = items[idx];
      blobItems.push(item);
      if (item.x < minX) minX = item.x;
      if (item.y < minY) minY = item.y;
    }
    blobs.push({ minX, minY, items: blobItems });
  }

  // Sort blobs in reading order: top-to-bottom, then left-to-right.
  // Use a tolerance band for "same row" detection (half of median font size).
  const rowTolerance = unit * 0.5;
  blobs.sort((a, b) => {
    const yDiff = a.minY - b.minY;
    if (Math.abs(yDiff) > rowTolerance) return yDiff;
    return a.minX - b.minX;
  });

  // Within each blob, reconstruct text: sort items into lines, then join.
  const blobTexts: string[] = [];

  for (const blob of blobs) {
    const text = reconstructBlobText(blob.items, unit);
    if (text.trim().length > 0) {
      blobTexts.push(text);
    }
  }

  return blobTexts.join("\n\n");
}

/**
 * Reconstruct readable text from a blob's text items.
 * Groups items into lines by y-coordinate, then joins words with spaces.
 */
function reconstructBlobText(items: TextItem[], unit: number): string {
  if (items.length === 0) return "";

  // Sort by y then x
  const sorted = [...items].sort((a, b) => {
    const yDiff = a.y - b.y;
    if (Math.abs(yDiff) > unit * 0.3) return yDiff;
    return a.x - b.x;
  });

  // Group into lines: items within 30% of unit vertically are on the same line
  const lines: TextItem[][] = [];
  let currentLine: TextItem[] = [sorted[0]];
  let lineY = sorted[0].y;

  for (let i = 1; i < sorted.length; i++) {
    const item = sorted[i];
    if (Math.abs(item.y - lineY) <= unit * 0.3) {
      currentLine.push(item);
    } else {
      lines.push(currentLine);
      currentLine = [item];
      lineY = item.y;
    }
  }
  lines.push(currentLine);

  // For each line, sort by x and join with appropriate spacing
  const lineTexts: string[] = [];
  for (const line of lines) {
    line.sort((a, b) => a.x - b.x);

    let lineStr = "";
    for (let i = 0; i < line.length; i++) {
      const item = line[i];
      if (i > 0) {
        const prev = line[i - 1];
        const gap = item.x - (prev.x + prev.w);
        // If gap is larger than a typical space (~0.25 * font size), add space
        if (gap > unit * 0.15) {
          lineStr += " ";
        }
      }
      lineStr += item.str;
    }
    lineTexts.push(lineStr);
  }

  return lineTexts.join("\n");
}
