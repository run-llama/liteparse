// E2E tests for pool mode (process-isolated workers with hard timeouts).
// Run with: npm test (node --test tests/)

import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
import { readFileSync, writeFileSync, mkdtempSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { LiteParse, ParseTimeoutError } from "../dist/lib.js";

const SAMPLE_PDF = fileURLToPath(
  new URL("../../../integration_tests_data/sample.pdf", import.meta.url),
);
const CFG = { quiet: true, ocrEnabled: false, outputFormat: "text" };

const HANG_HOOK = pathToFileURL(
  fileURLToPath(new URL("./support/hang-worker-replies.mjs", import.meta.url)),
).href;

/**
 * Builds a pool whose initial workers never answer a parse (see
 * support/hang-worker-replies.mjs), so every parse they get runs into the
 * deadline. The pool forks its workers in the constructor and they inherit
 * process.env at that moment; NODE_OPTIONS is restored straight after, so
 * workers spawned later, such as the replacement for a killed worker, are
 * ordinary.
 */
function poolWithHangingWorkers(options) {
  const saved = process.env.NODE_OPTIONS;
  process.env.NODE_OPTIONS = `${saved ?? ""} --import=${HANG_HOOK}`.trim();
  try {
    return new LiteParse(options);
  } finally {
    if (saved === undefined) delete process.env.NODE_OPTIONS;
    else process.env.NODE_OPTIONS = saved;
  }
}

test("pool matches in-process results", { skip: !existsSync(SAMPLE_PDF) }, async () => {
  const inproc = new LiteParse(CFG);
  const expected = await inproc.parse(SAMPLE_PDF);
  const pooled = new LiteParse({ ...CFG, poolSize: 2 });
  try {
    const fromPath = await pooled.parse(SAMPLE_PDF);
    const fromBytes = await pooled.parse(readFileSync(SAMPLE_PDF));
    assert.equal(fromPath.text, expected.text);
    assert.equal(fromBytes.text, expected.text);
    assert.equal(fromPath.pages.length, expected.pages.length);
  } finally {
    pooled.close();
  }
});

test("parseTimeoutMs requires poolSize", () => {
  assert.throws(() => new LiteParse({ parseTimeoutMs: 5000 }), /poolSize/);
});

for (const timeout of [
  Infinity,
  2_147_483_648,
  Number.MAX_SAFE_INTEGER,
  Number.MAX_VALUE,
]) {
  test(`parseTimeoutMs rejects a deadline outside the timer range: ${timeout}`, () => {
    let parser;
    try {
      assert.throws(() => {
        parser = new LiteParse({ ...CFG, poolSize: 1, parseTimeoutMs: timeout });
      }, /parseTimeoutMs/);
    } finally {
      parser?.close();
    }
  });
}

test("parseTimeoutMs accepts the maximum supported timer delay", async () => {
  const parser = new LiteParse({ ...CFG, poolSize: 1, parseTimeoutMs: 2_147_483_647 });
  try {
    const result = await parser.parse(gridPdf(1));
    assert.equal(result.pages.length, 1);
  } finally {
    parser.close();
  }
});

test("timeout kills worker and pool recovers", { skip: !existsSync(SAMPLE_PDF) }, async () => {
  const pooled = poolWithHangingWorkers({ ...CFG, poolSize: 1, parseTimeoutMs: 500 });
  try {
    await pooled.warmUp();
    await assert.rejects(pooled.parse(readFileSync(SAMPLE_PDF)), (e) => {
      assert.ok(e instanceof ParseTimeoutError);
      assert.equal(e.timeoutMs, 500);
      assert.match(e.source, /bytes/);
      return true;
    });
    // The replacement worker must serve subsequent parses.
    const result = await pooled.parse(SAMPLE_PDF);
    assert.ok(result.text.length > 0);
  } finally {
    pooled.close();
  }
});

test("timeout names the file", async () => {
  const dir = mkdtempSync(join(tmpdir(), "liteparse-pool-"));
  const slowPath = join(dir, "rogue.pdf");
  writeFileSync(slowPath, "%PDF-1.4\n%%EOF\n");
  const pooled = poolWithHangingWorkers({ ...CFG, poolSize: 1, parseTimeoutMs: 500 });
  try {
    await assert.rejects(pooled.parse(slowPath), (e) => {
      assert.ok(e instanceof ParseTimeoutError);
      assert.equal(e.source, slowPath);
      return true;
    });
  } finally {
    pooled.close();
  }
});

test("parse errors propagate from the worker", async () => {
  const pooled = new LiteParse({ ...CFG, poolSize: 1 });
  try {
    await assert.rejects(
      pooled.parse(Buffer.from("%PDF-1.4 not actually a pdf")),
      (e) => !(e instanceof ParseTimeoutError),
    );
    // Worker survives a failed parse and serves the next one.
    if (existsSync(SAMPLE_PDF)) {
      const result = await pooled.parse(SAMPLE_PDF);
      assert.ok(result.text.length > 0);
    }
  } finally {
    pooled.close();
  }
});

test("close() shuts down the pool", { skip: !existsSync(SAMPLE_PDF) }, async () => {
  const pooled = new LiteParse({ ...CFG, poolSize: 2 });
  await pooled.parse(SAMPLE_PDF);
  pooled.close();
  await assert.rejects(pooled.parse(SAMPLE_PDF), /closed/);
  pooled.close(); // idempotent
});

test("CJS build resolves and forks the pool worker", { skip: !existsSync(SAMPLE_PDF) }, async () => {
  // The worker file is located relative to import.meta.url, which tsup shims
  // in the .cjs bundle — this guards the fork path for require() consumers.
  const { createRequire } = await import("node:module");
  const require = createRequire(import.meta.url);
  const cjs = require("../dist/lib.cjs");
  const pooled = new cjs.LiteParse({ ...CFG, poolSize: 1, parseTimeoutMs: 30_000 });
  try {
    const result = await pooled.parse(SAMPLE_PDF);
    assert.ok(result.text.length > 0);
  } finally {
    pooled.close();
  }
});

test("pooled parses run concurrently", { skip: !existsSync(SAMPLE_PDF) }, async () => {
  const pooled = new LiteParse({ ...CFG, poolSize: 4 });
  try {
    await pooled.warmUp();
    const results = await Promise.all(
      Array.from({ length: 8 }, () => pooled.parse(SAMPLE_PDF)),
    );
    assert.equal(results.length, 8);
    for (const result of results) assert.ok(result.text.length > 0);
  } finally {
    pooled.close();
  }
});
