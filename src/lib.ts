/**
 * LiteParse - Programmatic API
 *
 * Use this entry point for importing LiteParse as a library in your projects.
 *
 * @example
 * ```typescript
 * import { LiteParse } from 'liteparse';
 *
 * const parser = new LiteParse({ ocrEnabled: true });
 * const result = await parser.parse('document.pdf');
 * console.log(result.text);
 * ```
 */

// Export main parser class and types
export { LiteParse } from './core/parser.js';
export type {
  LiteParseConfig,
  OutputFormat,
  ParseResult,
  PageResult,
  BoundingBox,
  TextItem,
  ImageItem,
  TableItem
} from './core/types.js';
