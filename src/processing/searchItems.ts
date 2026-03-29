import { JsonTextItem, SearchItemsOptions, TextLine } from "../core/types.js";

/**
 * Search text items for matches, returning synthetic merged items for each match.
 *
 * For phrase searches, consecutive text items are concatenated and searched.
 * When a phrase spans multiple items, the result is a single merged item with
 * combined bounding box and the matched text. Font metadata is taken from the
 * first matched item.
 *
 * @example
 * ```typescript
 * import { LiteParse, searchItems } from "@llamaindex/liteparse";
 *
 * const parser = new LiteParse({ outputFormat: "json" });
 * const result = await parser.parse("report.pdf");
 *
 * for (const page of result.json.pages) {
 *   const matches = searchItems(page.textItems, { phrase: "0°C to 70°C" });
 *   for (const match of matches) {
 *     console.log(`Found "${match.text}" at (${match.x}, ${match.y})`);
 *   }
 * }
 * ```
 */
export function searchItems(items: JsonTextItem[], options: SearchItemsOptions): JsonTextItem[] {
  const results: JsonTextItem[] = [];
  const caseSensitive = options.caseSensitive ?? false;
  const normalize = caseSensitive ? (s: string) => s : (s: string) => s.toLowerCase();
  const q = normalize(options.phrase);

  // Pre-compute separator between each pair of adjacent items.
  // If two items are on the same line and the next item starts where the
  // previous one ends (spatially adjacent), they are joined without a space.
  // Otherwise a space is inserted.
  const seps: string[] = new Array(items.length).fill("");
  for (let i = 1; i < items.length; i++) {
    const prev = items[i - 1];
    const cur = items[i];
    const fontSize = prev.fontSize ?? cur.fontSize ?? 12;
    const sameLine = Math.abs(cur.y - prev.y) < fontSize * 0.5;
    const gap = cur.x - (prev.x + prev.width);
    seps[i] = sameLine && gap <= fontSize * 0.3 ? "" : " ";
  }

  let start = 0;
  while (start < items.length) {
    let combined = "";
    let found = false;
    for (let end = start; end < items.length; end++) {
      if (end > start) combined += seps[end];
      combined += items[end].text;
      if (normalize(combined).includes(q)) {
        // Narrow from the left: drop leading items that aren't part of the match
        let narrowed = combined;
        let s = start;
        while (s < end) {
          const without = narrowed.slice(items[s].text.length + seps[s + 1].length);
          if (normalize(without).includes(q)) {
            narrowed = without;
            s++;
          } else {
            break;
          }
        }

        // Merge bounding boxes of the matched items
        const matched = items.slice(s, end + 1);
        const x = Math.min(...matched.map((m) => m.x));
        const y = Math.min(...matched.map((m) => m.y));
        const x2 = Math.max(...matched.map((m) => m.x + m.width));
        const y2 = Math.max(...matched.map((m) => m.y + m.height));

        results.push({
          text: options.phrase,
          x,
          y,
          width: x2 - x,
          height: y2 - y,
          fontName: matched[0].fontName,
          fontSize: matched[0].fontSize,
        });

        // Advance past the match to avoid duplicates
        start = end + 1;
        found = true;
        break;
      }
      // Stop expanding if the combined text is already much longer than the query
      if (combined.length > q.length * 2) break;
    }
    if (!found) start++;
  }

  return results;
}

/**
 * Search text lines for a phrase, returning all lines that participate in the match.
 *
 * Handles phrases that span multiple consecutive lines by concatenating adjacent
 * line texts with `"\n"` separators (matching the format of {@link ParsedPage.text}).
 * Returns the individual {@link TextLine} entries so the consumer can highlight
 * each line separately with its own bounding box.
 *
 * Requires {@link LiteParseConfig.textLineTracking} to be enabled during parsing.
 *
 * @example
 * ```typescript
 * import { LiteParse, searchTextLines } from "@llamaindex/liteparse";
 *
 * const parser = new LiteParse({ textLineTracking: true, outputFormat: "json" });
 * const result = await parser.parse("contract.pdf");
 *
 * for (const page of result.pages) {
 *   const matches = searchTextLines(page.textLines!, { phrase: "governing law" });
 *   for (const line of matches) {
 *     console.log(`Line "${line.text}" at (${line.bbox.x}, ${line.bbox.y})`);
 *   }
 * }
 * ```
 */
export function searchTextLines(textLines: TextLine[], options: SearchItemsOptions): TextLine[] {
  const caseSensitive = options.caseSensitive ?? false;
  const normalize = caseSensitive ? (s: string) => s : (s: string) => s.toLowerCase();
  const query = normalize(options.phrase);

  const matchedIndices = new Set<number>();

  let start = 0;
  while (start < textLines.length) {
    let combined = "";
    let found = false;
    for (let end = start; end < textLines.length; end++) {
      if (end > start) combined += "\n";
      combined += textLines[end].text;

      if (normalize(combined).includes(query)) {
        // Narrow from the left: drop leading lines that aren't part of the match
        let narrowed = combined;
        let s = start;
        while (s < end) {
          const without = narrowed.slice(textLines[s].text.length + 1); // +1 for "\n"
          if (normalize(without).includes(query)) {
            narrowed = without;
            s++;
          } else {
            break;
          }
        }

        for (let i = s; i <= end; i++) {
          matchedIndices.add(i);
        }
        start = end + 1;
        found = true;
        break;
      }

      // Stop expanding if combined text is much longer than query
      if (combined.length > query.length * 3) break;
    }
    if (!found) start++;
  }

  return Array.from(matchedIndices)
    .sort((a, b) => a - b)
    .map((i) => textLines[i]);
}
