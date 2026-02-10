import { Path } from '../engines/pdf/interface.js';
import { LiteParseConfig } from '../core/types.js';

export interface Line {
  x: number;
  y1: number;
  y2: number;
  width: number;
  height: number;
}

export interface VerticalLine extends Line {}
export interface HorizontalLine extends Line {}

/**
 * Extract vertical and horizontal lines from PDF paths
 * This is used to detect table borders and layout structure
 */
export function getVerticalAndHorizontalLinesFromPagePaths(
  _config: LiteParseConfig,
  paths: Path[]
): { verticalLines: VerticalLine[]; horizontalLines: HorizontalLine[] } {
  const verticalLines: VerticalLine[] = [];
  const horizontalLines: HorizontalLine[] = [];

  // Simplified version - full implementation would analyze path types
  // and extract line segments from rectangles and line paths

  for (const path of paths) {
    if (path.type === 'line' && path.points.length >= 2) {
      const [start, end] = path.points;
      const [x1, y1] = start;
      const [x2, y2] = end;

      // Check if vertical (x coordinates similar)
      if (Math.abs(x2 - x1) < 2) {
        verticalLines.push({
          x: (x1 + x2) / 2,
          y1: Math.min(y1, y2),
          y2: Math.max(y1, y2),
          width: path.width || 1,
          height: Math.abs(y2 - y1),
        });
      }
      // Check if horizontal (y coordinates similar)
      else if (Math.abs(y2 - y1) < 2) {
        horizontalLines.push({
          x: Math.min(x1, x2),
          y1: (y1 + y2) / 2,
          y2: (y1 + y2) / 2,
          width: Math.abs(x2 - x1),
          height: path.width || 1,
        });
      }
    }
    // Handle rectangles as potential table borders
    else if (path.type === 'rectangle' && path.points.length >= 4) {
      const xs = path.points.map((p) => p[0]);
      const ys = path.points.map((p) => p[1]);
      const minX = Math.min(...xs);
      const maxX = Math.max(...xs);
      const minY = Math.min(...ys);
      const maxY = Math.max(...ys);

      // Add vertical edges
      verticalLines.push(
        {
          x: minX,
          y1: minY,
          y2: maxY,
          width: path.width || 1,
          height: maxY - minY,
        },
        {
          x: maxX,
          y1: minY,
          y2: maxY,
          width: path.width || 1,
          height: maxY - minY,
        }
      );

      // Add horizontal edges
      horizontalLines.push(
        {
          x: minX,
          y1: minY,
          y2: minY,
          width: maxX - minX,
          height: path.width || 1,
        },
        {
          x: minX,
          y1: maxY,
          y2: maxY,
          width: maxX - minX,
          height: path.width || 1,
        }
      );
    }
  }

  return { verticalLines, horizontalLines };
}
