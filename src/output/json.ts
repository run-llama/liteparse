import { ParseResult, ParsedPage, ParseResultJson, JsonTextItem } from "../core/types.js";

/**
 * Build JSON output from parsed pages
 */
export function buildJSON(pages: ParsedPage[]): ParseResultJson {
  return {
    pages: pages.map((page) => ({
      page: page.pageNum,
      width: page.width,
      height: page.height,
      text: page.text,
      textItems: page.textItems.map((item) => {
        const result: JsonTextItem = {
          text: item.str,
          x: item.x,
          y: item.y,
          width: item.width,
          height: item.height,
          fontName: item.fontName,
          fontSize: item.fontSize,
          confidence: item.confidence ?? 1.0,
        };
        if (item.url !== undefined) {
          result.url = item.url;
        }
        return result;
      }),
      boundingBoxes: page.boundingBoxes || [],
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
