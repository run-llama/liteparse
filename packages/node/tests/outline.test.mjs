import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { LiteParse } from "@llamaindex/liteparse";

const fixture = resolve(dirname(fileURLToPath(import.meta.url)), "../../../integration_tests_data/outline.pdf");

test("returns the document outline for paths, buffers, and selected pages", async () => {
  const parser = new LiteParse({ ocrEnabled: false, quiet: true });
  const result = await parser.parse(fixture);
  assert.deepEqual(result.outline.map(({ level, title, pageIndex }) => [level, title, pageIndex]), [
    [1, "Chapter 1", 0],
    [2, "Section 1.1", 0],
    [1, "Chapter 2", 1],
  ]);
  for (const item of result.outline) {
    assert.ok(item.yPdf == null || typeof item.yPdf === "number");
  }
  assert.deepEqual((await parser.parse(readFileSync(fixture))).outline, result.outline);
  const selected = await new LiteParse({ ocrEnabled: false, quiet: true, targetPages: "1" }).parse(fixture);
  assert.equal(selected.pages.length, 1);
  assert.deepEqual(selected.outline, result.outline);
});

test("returns the document outline with every batch", async () => {
  const parser = new LiteParse({ ocrEnabled: false, quiet: true });
  const batches = [];
  for await (const batch of parser.parseBatches(fixture, { batchSize: 1 })) batches.push(batch);
  assert.equal(batches.length, 2);
  assert.deepEqual(batches[0].result.outline, batches[1].result.outline);
  assert.deepEqual(batches[0].result.outline.map((item) => item.title), ["Chapter 1", "Section 1.1", "Chapter 2"]);
});

test("returns an empty outline when the PDF has no bookmarks", async () => {
  const parser = new LiteParse({ ocrEnabled: false, quiet: true });
  assert.deepEqual((await parser.parse(resolve(dirname(fixture), "no_outline.pdf"))).outline, []);
});

test("CLI JSON includes the same outline with Rust field names", () => {
  const cli = resolve(dirname(fileURLToPath(import.meta.url)), "../dist/cli.js");
  const output = execFileSync(process.execPath, [cli, "parse", fixture, "--format", "json", "--no-ocr", "--quiet"], { encoding: "utf8" });
  const parsed = JSON.parse(output);
  assert.deepEqual(parsed.outline.map(({ level, title, page_index }) => [level, title, page_index]), [
    [1, "Chapter 1", 0], [2, "Section 1.1", 0], [1, "Chapter 2", 1],
  ]);
  assert.ok(Object.hasOwn(parsed.outline[0], "y_pdf"));
});
