/**
 * Edge runtime test for ocria-nano in the LiteParse WASM module: a Miniflare
 * (Cloudflare Workers) instance loads the WASM module and the ocria-nano models
 * (as data modules) and OCRs a PDF.
 *
 * Usage: OCRIA_MODELS=<ocria-nano model dir> [EXPECT_TEXT=<substring>] node scripts/edge-compat/wasm-ocria-test.mjs [file.pdf]
 *
 * Requires: miniflare (npm i -D miniflare)
 * Expects:  packages/wasm/pkg/ to contain the built WASM files
 */

import { Miniflare } from "miniflare";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(import.meta.url), "../../..");

const wasmPath = resolve(ROOT, "packages/wasm/pkg/liteparse_wasm_bg.wasm");
const gluePath = resolve(ROOT, "packages/wasm/pkg/liteparse_wasm.js");
const pdfPath = process.argv[2] ?? resolve(ROOT, "demo/docs/apple-10k-2024.pdf");
const modelDir = process.env.OCRIA_MODELS;
if (!modelDir) {
  console.log("SKIP: set OCRIA_MODELS to an ocria-nano model directory");
  process.exit(0);
}
const MODEL_FILES = ["det.onnx", "det.meta.json", "router.onnx", "rec_latin.onnx", "rec_latin.charset.json", "tablenet.bin"];

// Build a self-contained worker script.
// The generated glue exports an async `init()` that fetches the .wasm via URL.
// In edge runtimes, we import the WASM as a module and use `initSync` instead.
let glueSource = readFileSync(gluePath, "utf-8");

// Strip default export (the async init / fetch-based loader)
glueSource = glueSource.replace(/export\s*\{[^}]*__wbg_init\s+as\s+default[^}]*\};?/g, "");
glueSource = glueSource.replace(/export\s+default\s+__wbg_init\s*;?/g, "");
glueSource = glueSource.replace(/export\s*\{\s*initSync\s*(?:,\s*__wbg_init\s+as\s+default\s*)?\}\s*;?/g, "");

const workerScript = `
import __liteparse_wasm_mod from "liteparse.wasm";
${MODEL_FILES.map((f, i) => `import m${i} from "${f}";`).join("\n")}

${glueSource}

const files = { ${MODEL_FILES.map((f, i) => `"${f}": m${i}`).join(", ")} };
const u8 = (f) => new Uint8Array(files[f]);
let ocria;

export default {
  async fetch(request) {
    try {
      initSync({ module: __liteparse_wasm_mod });
      const t0 = Date.now();
      ocria ??= OcriaNano.load({
        det: u8("det.onnx"), detMeta: u8("det.meta.json"), router: u8("router.onnx"),
        recLatin: { onnx: u8("rec_latin.onnx"), charset: u8("rec_latin.charset.json") },
        tablenet: u8("tablenet.bin"),
      });
      const t1 = Date.now();
      const parser = LiteParse.withOcriaNano({ ocrEnabled: true, outputFormat: "markdown" }, ocria);
      const result = await parser.parse(new Uint8Array(await request.arrayBuffer()));
      return Response.json({ ok: true, pages: result.pages.length, textLength: result.text.length,
        loadMs: t1 - t0, parseMs: Date.now() - t1, head: result.text.slice(0, 200) });
    } catch (err) {
      return Response.json({ ok: false, error: err.message || String(err), stack: err.stack }, { status: 500 });
    }
  }
};
`;

async function main() {
  console.log("Starting Miniflare (Cloudflare Workers runtime) with ocria-nano...");
  const mf = new Miniflare({
    modules: [
      { type: "ESModule", path: "worker.mjs", contents: workerScript },
      { type: "CompiledWasm", path: "liteparse.wasm", contents: readFileSync(wasmPath) },
      ...MODEL_FILES.map((f) => ({ type: "Data", path: f, contents: readFileSync(resolve(modelDir, f)) })),
    ],
    compatibilityDate: "2024-01-01",
  });
  try {
    const response = await mf.dispatchFetch("http://localhost/parse", { method: "POST", body: readFileSync(pdfPath) });
    const result = await response.json();
    if (!result.ok) {
      console.error(`FAIL: ${result.error}`);
      if (result.stack) console.error(result.stack);
      process.exit(1);
    }
    if (result.textLength < 20) {
      console.error(`FAIL: text too short: ${result.textLength} chars`);
      process.exit(1);
    }
    const expected = process.env.EXPECT_TEXT;
    if (expected && !result.head.includes(expected)) {
      console.error(`FAIL: expected ${JSON.stringify(expected)} in the OCR text: ${JSON.stringify(result.head)}`);
      process.exit(1);
    }
    console.log(`PASS: ${result.pages} pages, ${result.textLength} chars, models ${result.loadMs} ms, parse ${result.parseMs} ms`);
    console.log(result.head);
  } finally {
    await mf.dispose();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
