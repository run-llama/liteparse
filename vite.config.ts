import { defineConfig, type Plugin } from "vite";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = (p: string) => resolve(fileURLToPath(new URL(".", import.meta.url)), p);

const FILE_REDIRECTS: Array<{ match: RegExp; target: string }> = [
  { match: /\/engines\/pdf\/pdfium-renderer(\.js|\.ts)?$/, target: here("web/pdfjs-renderer.ts") },
  { match: /\/engines\/pdf\/pdfjsImporter(\.js|\.ts)?$/, target: here("web/pdfjsImporter.ts") },
  { match: /\/engines\/ocr\/http-simple(\.js|\.ts)?$/, target: here("web/stubs/http-simple.ts") },
  { match: /\/conversion\/convertToPdf(\.js|\.ts)?$/, target: here("web/stubs/convertToPdf.ts") },
  {
    match: /\/processing\/gridDebugLogger(\.js|\.ts)?$/,
    target: here("web/stubs/gridDebugLogger.ts"),
  },
  {
    match: /\/processing\/gridVisualizer(\.js|\.ts)?$/,
    target: here("web/stubs/gridVisualizer.ts"),
  },
];

// Rewrites relative imports of Node-only source files to their browser
// replacements. Runs at resolveId so it catches `../engines/pdf/pdfium-renderer.js`
// from parser.ts and `./pdfium-renderer.js` from pdfjs.ts alike.
function liteparseNodeRedirects(): Plugin {
  return {
    name: "liteparse-node-redirects",
    enforce: "pre",
    async resolveId(source, importer) {
      if (!importer) return null;
      // Resolve relative to the importer so we match on absolute path
      const importerDir = dirname(importer);
      const absolutePath = source.startsWith(".") ? resolve(importerDir, source) : source;
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
  root: "web",
  publicDir: here("public"),
  build: {
    outDir: "../dist-web",
    emptyOutDir: true,
    target: "es2022",
  },
  server: {
    fs: { allow: [".."] },
  },
  optimizeDeps: {
    include: ["tesseract.js"],
  },
  plugins: [liteparseNodeRedirects()],
  resolve: {
    alias: [
      { find: "node:fs/promises", replacement: here("web/stubs/empty.ts") },
      { find: "node:fs", replacement: here("web/stubs/empty.ts") },
      { find: "node:url", replacement: here("web/stubs/node-url.ts") },
      { find: "node:path", replacement: here("web/stubs/node-path.ts") },
      { find: "node:os", replacement: here("web/stubs/empty.ts") },
      { find: "node:child_process", replacement: here("web/stubs/empty.ts") },
      { find: /^fs$/, replacement: here("web/stubs/empty.ts") },
      { find: /^fs\/promises$/, replacement: here("web/stubs/empty.ts") },
      { find: /^path$/, replacement: here("web/stubs/node-path.ts") },
      { find: /^os$/, replacement: here("web/stubs/empty.ts") },
      { find: /^child_process$/, replacement: here("web/stubs/empty.ts") },
      { find: "form-data", replacement: here("web/stubs/empty.ts") },
      { find: "axios", replacement: here("web/stubs/empty.ts") },
      { find: "file-type", replacement: here("web/stubs/file-type.ts") },
    ],
  },
  define: {
    "process.env.NODE_ENV": JSON.stringify("production"),
  },
});
