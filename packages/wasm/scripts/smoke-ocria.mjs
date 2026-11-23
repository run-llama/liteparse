// Smoke test: parse PDFs with ocria-nano in the WASM build (Node, web-target package).
//   OCRIA_MODELS=<ocria-nano model dir> node scripts/smoke-ocria.mjs file.pdf [...]
// Prints load / parse times and writes <file>.wasm.md next to each input.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const pkg = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "pkg");
const { initSync, LiteParse, OcriaNano } = await import(path.join(pkg, "liteparse_wasm.js"));
initSync({ module: fs.readFileSync(path.join(pkg, "liteparse_wasm_bg.wasm")) });

const dir = process.env.OCRIA_MODELS;
if (!dir) throw new Error("set OCRIA_MODELS to an ocria-nano model directory");
const read = (f) => (fs.existsSync(path.join(dir, f)) ? new Uint8Array(fs.readFileSync(path.join(dir, f))) : undefined);
const rec = (s) => (read(`rec_${s}.onnx`) ? { onnx: read(`rec_${s}.onnx`), charset: read(`rec_${s}.charset.json`), meta: read(`rec_${s}.meta.json`) } : undefined);

let t = performance.now();
const ocria = OcriaNano.load({
  det: read("det.onnx"), detMeta: read("det.meta.json"), router: read("router.onnx"),
  recLatin: rec("latin"), recCjk: rec("cjk"), recRtl: rec("rtl"), recWorld: rec("world"),
  tablenet: read("tablenet.bin"),
});
console.log(`models loaded in ${(performance.now() - t).toFixed(0)} ms (LayoutNet: ${ocria.hasLayoutModel})`);
const parser = LiteParse.withOcriaNano({ ocrEnabled: true, outputFormat: "markdown", keepHeadersFooters: true, extractLinks: false }, ocria);
for (const f of process.argv.slice(2)) {
  t = performance.now();
  const r = await parser.parse(new Uint8Array(fs.readFileSync(f)));
  fs.writeFileSync(`${f}.wasm.md`, r.text);
  console.log(`${path.basename(f)}: ${(performance.now() - t).toFixed(0)} ms, ${r.text.length} chars`);
}
