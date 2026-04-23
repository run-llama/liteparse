import { defineConfig } from "vite";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = (p: string) => resolve(fileURLToPath(new URL(".", import.meta.url)), p);

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
    exclude: ["tesseract.js"],
  },
  resolve: {
    alias: [
      // Node-only modules that show up in the import graph but are never executed in browser
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
      // File-level redirects
      {
        find: here("src/engines/pdf/pdfium-renderer.ts"),
        replacement: here("web/pdfjs-renderer.ts"),
      },
      {
        find: here("src/engines/pdf/pdfjsImporter.ts"),
        replacement: here("web/pdfjsImporter.ts"),
      },
      {
        find: here("src/engines/ocr/http-simple.ts"),
        replacement: here("web/stubs/http-simple.ts"),
      },
      {
        find: here("src/conversion/convertToPdf.ts"),
        replacement: here("web/stubs/convertToPdf.ts"),
      },
      {
        find: here("src/processing/gridDebugLogger.ts"),
        replacement: here("web/stubs/gridDebugLogger.ts"),
      },
      {
        find: here("src/processing/gridVisualizer.ts"),
        replacement: here("web/stubs/gridVisualizer.ts"),
      },
    ],
  },
  define: {
    "process.env.NODE_ENV": JSON.stringify("production"),
  },
});
