import type { ParsedPage } from "./lib.js";

/** Format parsed pages as plain text with page headers. */
export function formatText(
  pages: readonly Pick<ParsedPage, "pageNum" | "text">[],
): string {
  return pages
    .map((page) => `\n--- Page ${page.pageNum} ---\n${page.text}`)
    .join("\n\n");
}
