import { ParseResult, ParsedPage } from '../core/types.js';

/**
 * Format pages as plain text
 */
export function formatText(result: ParseResult): string {
  const pageTexts = result.pages.map((page) => {
    const header = `\n--- Page ${page.pageNum} ---\n`;
    return header + page.text;
  });

  return pageTexts.join('\n\n');
}

/**
 * Format single page as text
 */
export function formatPageText(page: ParsedPage): string {
  return page.text;
}
