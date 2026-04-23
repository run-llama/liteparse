// Browser-safe public entrypoint. Vite aliases redirect Node-only modules
// to stubs so importing LiteParse pulls in the browser-compatible subset.

export { LiteParse } from "../src/core/parser.js";
export type {
  LiteParseConfig,
  ParseResult,
  ParseResultJson,
  ParsedPage,
  TextItem,
  JsonTextItem,
} from "../src/core/types.js";
