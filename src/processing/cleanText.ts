import { ParsedPage, LiteParseConfig } from '../core/types.js';

/**
 * Detect and remove margins from a single page.
 * Removes:
 * - Left margin (consistent leading whitespace)
 * - Top margin (empty lines at start)
 * - Bottom margin (empty lines at end)
 * - Right margin (trailing whitespace on each line)
 *
 * Ported from llamaparse's detectAndRemoveMargin function.
 */
function detectAndRemoveMarginOnPage(page: ParsedPage): void {
  const lines = page.text.split('\n');

  let minX: number | undefined = undefined;
  let minY: number | undefined = undefined;
  let maxY: number | undefined = undefined;

  // Find margins
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    if (line.trim().length === 0) {
      continue;
    }

    // Find first non-whitespace character position (left margin)
    const x = line.search(/\S/);
    if (minX === undefined || x < minX) {
      minX = x;
    }

    // First non-empty line (top margin)
    if (minY === undefined) {
      minY = index;
    }

    // Last non-empty line (bottom margin)
    maxY = index;
  }

  // If page is entirely empty, just return
  if (minX === undefined || minY === undefined || maxY === undefined) {
    page.text = '';
    return;
  }

  // Remove margins
  const newLines: string[] = [];
  for (let index = 0; index < lines.length; index++) {
    // Skip lines before first content (top margin) or after last content (bottom margin)
    if (index < minY || index > maxY) {
      continue;
    }

    let line = lines[index];

    // Remove trailing whitespace (right margin)
    line = line.trimEnd();

    // Remove left margin
    newLines.push(line.slice(minX));
  }

  page.text = newLines.join('\n');
}

/**
 * Detect and remove margins from all pages.
 * Processes each page independently.
 */
function detectAndRemoveMargin(pages: ParsedPage[]): void {
  for (const page of pages) {
    detectAndRemoveMarginOnPage(page);
  }
}

/**
 * Clean raw text output - removes margins, null characters
 */
export function cleanRawText(
  pages: ParsedPage[],
  _config: LiteParseConfig
): void {
  // Remove margins (per-page)
  detectAndRemoveMargin(pages);

  // Remove null characters
  for (const page of pages) {
    page.text = page.text.replace(/\u0000/g, ' ');
  }
}
