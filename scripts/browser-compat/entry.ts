// Minimal entry point that exercises the LiteParse browser import path.
// If this bundles successfully with Vite, the browser compat config works.
import { LiteParse } from "../../src/lib.js";
export type { ParseResult, LiteParseConfig } from "../../src/lib.js";

// Instantiate to verify the constructor doesn't crash at bundle time
const parser = new LiteParse({ ocrEnabled: false, outputFormat: "text" });

// Export so Vite doesn't tree-shake it away
export { parser, LiteParse };
