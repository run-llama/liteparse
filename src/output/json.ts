import { ParseResult, ParsedPage, ParseResultJson } from "../core/types.js";

/**
 * Build JSON output from parsed pages
 */
export function buildJSON(pages: ParsedPage[]): ParseResultJson {
  return {
    pages: pages.map((page) => {
      const entry: ParseResultJson["pages"][number] = {
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
          confidence: item.confidence ?? 1.0,
        })),
        boundingBoxes: page.boundingBoxes || [],
      };
      if (page.textLines) {
        entry.textLines = page.textLines.map((tl) => ({
          text: tl.text,
          x: tl.bbox.x,
          y: tl.bbox.y,
          width: tl.bbox.w,
          height: tl.bbox.h,
          lineNumber: tl.lineNumber,
        }));
      }
      return entry;
    }),
  };
}

/**
 * Format result as JSON string
 */
export function formatJSON(result: ParseResult): string {
  const jsonData = buildJSON(result.pages);
  return JSON.stringify(jsonData, null, 2);
}
