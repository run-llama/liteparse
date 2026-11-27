// E2E tests for pool mode (process-isolated workers with hard timeouts).
// Run with: npm test (node --test tests/)

import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
import { readFileSync, writeFileSync, mkdtempSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

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

for (const { name, args, stdin = false, inline = false } of [
  { name: "module stdin", args: ["--input-type=module"], stdin: true },
  { name: "CommonJS stdin", args: ["--input-type=commonjs"], stdin: true },
  { name: "module eval", args: ["--input-type=module", "--eval"] },
  { name: "separate input type", args: ["--input-type", "module", "-e"] },
  { name: "CommonJS print/eval", args: ["--input-type=commonjs", "-pe"] },
  { name: "inline eval", args: ["--input-type=module"], inline: true },
  { name: "ordinary eval", args: ["-e"] },
  { name: "ordinary print", args: ["-p"] },
]) {
  test(`pool starts and parses from ${name}`, () => {
    const code = `
      if (process.env.LITEPARSE_TEST_EVAL_GUARD === "1") {
        throw new Error("parent eval source ran in a worker");
      }
      process.env.LITEPARSE_TEST_EVAL_GUARD = "1";
      (async () => {
        const { LiteParse } = await import(${JSON.stringify(new URL("../dist/lib.js", import.meta.url).href)});
        const { deflateSync } = await import("node:zlib");
        const gridPdf = ${gridPdf.toString()};
        const originalArgs = JSON.stringify(process.execArgv);
        const parser = new LiteParse({ quiet: true, ocrEnabled: false, poolSize: 1 });
        try {
          await parser.warmUp();
          const result = await parser.parse(gridPdf(1));
          console.log("POOL_RESULT=" + JSON.stringify({
            pages: result.pages.length,
            argsUnchanged: originalArgs === JSON.stringify(process.execArgv),
          }));
        } finally {
          parser.close();
        }
      })().catch(error => { console.error(error); process.exitCode = 1; });
    `;
    const child = spawnSync(
      process.execPath,
      [...args, ...(stdin ? [] : [inline ? `--eval=${code}` : code])],
      {
        input: stdin ? code : undefined,
        encoding: "utf8",
        timeout: 10_000,
        env: { ...process.env, LITEPARSE_TEST_EVAL_GUARD: "" },
      },
    );
    assert.ifError(child.error);
    assert.equal(child.status, 0, child.stderr);
    assert.match(child.stdout, /POOL_RESULT=\{"pages":1,"argsUnchanged":true\}/);
  });
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
