import { ParseResult, ParsedPage } from '../core/types.js';

/**
 * Build JSON output from parsed pages
 */
export function buildJSON(pages: ParsedPage[]): any {
  return {
    pages: pages.map((page) => ({
      page: page.pageNum,
      width: page.width,
      height: page.height,
      text: page.text,
      textItems: page.textItems.map((item) => ({
        text: item.str,
        x: item.x,
        y: item.y,
        width: item.width,
        height: item.height,
        fontName: item.fontName,
        fontSize: item.fontSize,
      })),
      boundingBoxes: page.boundingBoxes || [],
      tables: page.tables || [],
      images: page.images || [],
    })),
  };
}

/**
 * Format result as JSON string
 */
export function formatJSON(result: ParseResult): string {
  const jsonData = buildJSON(result.pages);
  return JSON.stringify(jsonData, null, 2);
}
