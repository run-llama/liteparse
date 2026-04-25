/**
 * Minimal Vite config that verifies LiteParse can be bundled for the browser.
 * This mirrors the recommended configuration from the README.
 *
 * Run: npx vite build --config scripts/browser-compat/vite.config.ts
 */
import { defineConfig, type Plugin } from "vite";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const here = (p: string) => resolve(root, p);

// Node-only source files that must be replaced with browser stubs.
const FILE_REDIRECTS: Array<{ match: RegExp; target: string }> = [
  {
    match: /\/engines\/pdf\/pdfium-renderer(\.js|\.ts)?$/,
    target: here("scripts/browser-compat/stubs/pdfium-renderer.ts"),
  },
  {
    match: /\/engines\/pdf\/pdfjsImporter(\.js|\.ts)?$/,
    target: here("scripts/browser-compat/stubs/pdfjsImporter.ts"),
  },
  {
    match: /\/engines\/ocr\/http-simple(\.js|\.ts)?$/,
    target: here("scripts/browser-compat/stubs/http-simple.ts"),
  },
  {
    match: /\/conversion\/convertToPdf(\.js|\.ts)?$/,
    target: here("scripts/browser-compat/stubs/convertToPdf.ts"),
  },
  {
    match: /\/processing\/gridDebugLogger(\.js|\.ts)?$/,
    target: here("scripts/browser-compat/stubs/gridDebugLogger.ts"),
  },
  {
    match: /\/processing\/gridVisualizer(\.js|\.ts)?$/,
    target: here("scripts/browser-compat/stubs/gridVisualizer.ts"),
  },
];

function liteparseNodeRedirects(): Plugin {
  return {
    name: "liteparse-node-redirects",
    enforce: "pre",
    async resolveId(source, importer) {
      if (!importer) return null;
      const importerDir = dirname(importer);
      const absolutePath = source.startsWith(".")
        ? resolve(importerDir, source)
        : source;
      for (const { match, target } of FILE_REDIRECTS) {
        if (match.test(absolutePath) || match.test(source)) {
          return target;
        }
      }
      return null;
    },
  };
}

export default defineConfig({
  root: here("scripts/browser-compat"),
  build: {
    outDir: resolve(root, "dist-browser-compat"),
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: here("scripts/browser-compat/entry.ts"),
    },
  },
  server: {
    fs: { allow: [root] },
  },
  optimizeDeps: {
    include: ["tesseract.js"],
  },
  plugins: [liteparseNodeRedirects()],
  resolve: {
    alias: [
      { find: "node:fs/promises", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "node:fs", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "node:url", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "node:path", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "node:os", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "node:child_process", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: /^fs$/, replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: /^fs\/promises$/, replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: /^path$/, replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: /^os$/, replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: /^child_process$/, replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "form-data", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "axios", replacement: here("scripts/browser-compat/stubs/empty.ts") },
      { find: "file-type", replacement: here("scripts/browser-compat/stubs/file-type.ts") },
    ],
  },
  define: {
    "process.env.NODE_ENV": JSON.stringify("production"),
  },
});
