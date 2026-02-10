import { ParsedPage, LiteParseConfig } from '../core/types.js';

/**
 * Detect and remove margins from pages
 * Simplifies text by removing consistent left/right margins
 */
function detectAndRemoveMargin(pages: ParsedPage[]): void {
  if (!pages.length) return;

  // Find minimum left margin across all pages
  let minLeftMargin = Infinity;
  for (const page of pages) {
    const lines = page.text.split('\n');
    for (const line of lines) {
      if (line.trim().length > 0) {
        const leadingSpaces = line.length - line.trimStart().length;
        minLeftMargin = Math.min(minLeftMargin, leadingSpaces);
      }
    }
  }

  // Remove the common left margin
  if (minLeftMargin > 0 && minLeftMargin !== Infinity) {
    for (const page of pages) {
      const lines = page.text.split('\n');
      const trimmedLines = lines.map((line) => {
        if (line.length >= minLeftMargin) {
          return line.substring(minLeftMargin);
        }
        return line;
      });
      page.text = trimmedLines.join('\n');
    }
  }
}

/**
 * Clean raw text output - removes margins, null characters
 */
export function cleanRawText(
  pages: ParsedPage[],
  _config: LiteParseConfig
): void {
  // Remove margins
  detectAndRemoveMargin(pages);

  // Remove null characters
  for (const page of pages) {
    page.text = page.text.replace(/\u0000/g, ' ');
  }
}
