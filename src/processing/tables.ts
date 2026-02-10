import { Path } from '../engines/pdf/interface.js';
import { TextItem } from '../core/types.js';

export interface DetectedTable {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  rows: number;
  cols: number;
  cells?: TableCell[];
}

export interface TableCell {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  text: string;
  row: number;
  col: number;
}

interface Line {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  isHorizontal: boolean;
}

interface Intersection {
  x: number;
  y: number;
}

/**
 * Extract horizontal and vertical lines from paths
 */
function extractLines(paths: Path[]): { horizontal: Line[]; vertical: Line[] } {
  const horizontal: Line[] = [];
  const vertical: Line[] = [];
  const tolerance = 2; // Tolerance for determining if a line is horizontal/vertical

  for (const path of paths) {
    if (path.type === 'line' && path.points.length >= 2) {
      const [start, end] = path.points;
      const [x1, y1] = start;
      const [x2, y2] = end;

      const dx = Math.abs(x2 - x1);
      const dy = Math.abs(y2 - y1);

      // Horizontal line
      if (dy < tolerance && dx > 10) {
        horizontal.push({
          x1: Math.min(x1, x2),
          y1: (y1 + y2) / 2,
          x2: Math.max(x1, x2),
          y2: (y1 + y2) / 2,
          isHorizontal: true,
        });
      }
      // Vertical line
      else if (dx < tolerance && dy > 10) {
        vertical.push({
          x1: (x1 + x2) / 2,
          y1: Math.min(y1, y2),
          x2: (x1 + x2) / 2,
          y2: Math.max(y1, y2),
          isHorizontal: false,
        });
      }
    }
    // Extract lines from rectangles
    else if (path.type === 'rectangle' && path.points.length >= 4) {
      const xs = path.points.map((p) => p[0]);
      const ys = path.points.map((p) => p[1]);
      const minX = Math.min(...xs);
      const maxX = Math.max(...xs);
      const minY = Math.min(...ys);
      const maxY = Math.max(...ys);

      // Add edges as lines
      horizontal.push(
        { x1: minX, y1: minY, x2: maxX, y2: minY, isHorizontal: true },
        { x1: minX, y1: maxY, x2: maxX, y2: maxY, isHorizontal: true }
      );
      vertical.push(
        { x1: minX, y1: minY, x2: minX, y2: maxY, isHorizontal: false },
        { x1: maxX, y1: minY, x2: maxX, y2: maxY, isHorizontal: false }
      );
    }
  }

  return { horizontal, vertical };
}

/**
 * Find intersections between horizontal and vertical lines
 */
function findIntersections(
  horizontal: Line[],
  vertical: Line[],
  tolerance: number = 3
): Intersection[] {
  const intersections: Intersection[] = [];
  const seen = new Set<string>();

  for (const h of horizontal) {
    for (const v of vertical) {
      // Check if lines intersect
      const hY = h.y1;
      const vX = v.x1;

      // Check if intersection is within line bounds
      if (
        vX >= h.x1 - tolerance &&
        vX <= h.x2 + tolerance &&
        hY >= v.y1 - tolerance &&
        hY <= v.y2 + tolerance
      ) {
        const key = `${Math.round(vX)},${Math.round(hY)}`;
        if (!seen.has(key)) {
          intersections.push({ x: vX, y: hY });
          seen.add(key);
        }
      }
    }
  }

  return intersections;
}

/**
 * Cluster nearby lines together
 */
function clusterLines(lines: Line[], isHorizontal: boolean): Line[] {
  if (lines.length === 0) return [];

  // Sort lines by position
  const sorted = [...lines].sort((a, b) => {
    return isHorizontal ? a.y1 - b.y1 : a.x1 - b.x1;
  });

  const clusters: Line[] = [];
  let current = sorted[0];

  for (let i = 1; i < sorted.length; i++) {
    const line = sorted[i];
    const threshold = 5;

    // Check if lines are close enough to merge
    const distance = isHorizontal
      ? Math.abs(line.y1 - current.y1)
      : Math.abs(line.x1 - current.x1);

    if (distance < threshold) {
      // Merge lines
      if (isHorizontal) {
        current.x1 = Math.min(current.x1, line.x1);
        current.x2 = Math.max(current.x2, line.x2);
        current.y1 = (current.y1 + line.y1) / 2;
        current.y2 = current.y1;
      } else {
        current.y1 = Math.min(current.y1, line.y1);
        current.y2 = Math.max(current.y2, line.y2);
        current.x1 = (current.x1 + line.x1) / 2;
        current.x2 = current.x1;
      }
    } else {
      clusters.push(current);
      current = line;
    }
  }
  clusters.push(current);

  return clusters;
}

/**
 * Create cells from intersections
 */
function createCellsFromIntersections(
  intersections: Intersection[]
): TableCell[] {
  if (intersections.length < 4) return [];

  // Sort intersections
  const sorted = [...intersections].sort((a, b) => {
    if (Math.abs(a.y - b.y) < 2) return a.x - b.x;
    return a.y - b.y;
  });

  // Find unique x and y coordinates
  const xCoords = [...new Set(sorted.map((i) => Math.round(i.x)))].sort(
    (a, b) => a - b
  );
  const yCoords = [...new Set(sorted.map((i) => Math.round(i.y)))].sort(
    (a, b) => a - b
  );

  const cells: TableCell[] = [];

  // Create cells from grid
  for (let row = 0; row < yCoords.length - 1; row++) {
    for (let col = 0; col < xCoords.length - 1; col++) {
      cells.push({
        x1: xCoords[col],
        y1: yCoords[row],
        x2: xCoords[col + 1],
        y2: yCoords[row + 1],
        text: '',
        row,
        col,
      });
    }
  }

  return cells;
}

/**
 * Assign text items to cells
 */
function assignTextToCells(cells: TableCell[], textItems: TextItem[]): void {
  for (const cell of cells) {
    const cellText: string[] = [];

    for (const item of textItems) {
      // Check if text item overlaps with cell
      const itemCenterX = item.x + item.w / 2;
      const itemCenterY = item.y + item.h / 2;

      if (
        itemCenterX >= cell.x1 &&
        itemCenterX <= cell.x2 &&
        itemCenterY >= cell.y1 &&
        itemCenterY <= cell.y2
      ) {
        cellText.push(item.str);
      }
    }

    cell.text = cellText.join(' ').trim();
  }
}

/**
 * Validate if detected structure is a valid table
 */
function isValidTable(
  cells: TableCell[],
  minCells: number = 4,
  minRows: number = 2,
  minCols: number = 2
): boolean {
  if (cells.length < minCells) return false;

  const rows = new Set(cells.map((c) => c.row)).size;
  const cols = new Set(cells.map((c) => c.col)).size;

  return rows >= minRows && cols >= minCols;
}

/**
 * Detect tables using heuristic methods (outlined paths)
 * Uses line intersection detection to find table grid structures
 */
export function detectTables(
  paths: Path[],
  textItems: TextItem[] = []
): DetectedTable[] {
  const tables: DetectedTable[] = [];

  // Extract lines from paths
  const { horizontal, vertical } = extractLines(paths);

  if (horizontal.length < 2 || vertical.length < 2) {
    return tables;
  }

  // Cluster nearby lines to reduce noise
  const clusteredH = clusterLines(horizontal, true);
  const clusteredV = clusterLines(vertical, false);

  // Find intersections
  const intersections = findIntersections(clusteredH, clusteredV);

  if (intersections.length < 4) {
    return tables;
  }

  // Create cells from intersections
  const cells = createCellsFromIntersections(intersections);

  if (!isValidTable(cells)) {
    return tables;
  }

  // Assign text to cells
  assignTextToCells(cells, textItems);

  // Calculate table bounds
  const xCoords = cells.map((c) => c.x1).concat(cells.map((c) => c.x2));
  const yCoords = cells.map((c) => c.y1).concat(cells.map((c) => c.y2));

  const rows = new Set(cells.map((c) => c.row)).size;
  const cols = new Set(cells.map((c) => c.col)).size;

  tables.push({
    x1: Math.min(...xCoords),
    y1: Math.min(...yCoords),
    x2: Math.max(...xCoords),
    y2: Math.max(...yCoords),
    rows,
    cols,
    cells,
  });

  return tables;
}

/**
 * Extract table structure from detected table region
 */
export function extractTableStructure(
  table: DetectedTable,
  _paths: Path[]
): any {
  return {
    rows: table.rows,
    cols: table.cols,
    cells: table.cells || [],
  };
}
