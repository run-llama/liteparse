import { MarkupData } from '../core/types.js';

/**
 * Apply markup tags to text based on markup data
 * Returns text with inline markup tags that can be converted to markdown later
 */
export function applyMarkupTags(markup: MarkupData, text: string): string {
  let result = text;

  // Apply strikeout (~~text~~)
  if (markup.strikeout) {
    result = `~~${result}~~`;
  }

  // Apply underline (__text__)
  if (markup.underline) {
    result = `__${result}__`;
  }

  // Apply squiggly (wavy underline, use underline for now)
  if (markup.squiggly) {
    result = `__${result}__`;
  }

  // Apply highlight (==text==)
  if (markup.highlight) {
    result = `==${result}==`;
  }

  return result;
}
