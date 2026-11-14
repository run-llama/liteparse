// E2E tests for pool mode (process-isolated workers with hard timeouts).
// Run with: npm test (node --test tests/)

import { test } from "node:test";
import assert from "node:assert/strict";
import { deflateSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { writeFileSync, mkdtempSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

import { LiteParse, ParseTimeoutError } from "../dist/lib.js";

const SAMPLE_PDF = fileURLToPath(
  new URL("../../../integration_tests_data/sample.pdf", import.meta.url),
);
const CFG = { quiet: true, ocrEnabled: false, outputFormat: "text" };

/**
 * One page with a dense grid of tiny text objects. Parse time is superlinear
 * in the object count (PDFium's text-page assembly), which makes a small file
 * arbitrarily slow to parse — the same shape as the rogue documents that
 * stall prod pipelines, used here to exercise the timeout path.
 */
function gridPdf(nObjects) {
  const cols = Math.max(1, Math.floor(Math.sqrt(nObjects)));
  const parts = [];
  for (let i = 0; i < nObjects; i++) {
    const x = (20 + (i % cols) * 2.5).toFixed(1);
    const y = (770 - Math.floor(i / cols) * 2.5).toFixed(1);
    parts.push(`BT /F1 2 Tf ${x} ${y} Td (x) Tj ET`);
  }
  const stream = deflateSync(Buffer.from(parts.join("\n")));
  const width = Math.floor(40 + cols * 2.5);
  const height = Math.floor(40 + (nObjects / cols) * 2.5);
  const objects = [
    Buffer.from("<< /Type /Catalog /Pages 2 0 R >>"),
    Buffer.from("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
    Buffer.from(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${width} ${height}] ` +
        "/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    ),
    Buffer.from("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
    Buffer.concat([
      Buffer.from(`<< /Length ${stream.length} /Filter /FlateDecode >>\nstream\n`),
      stream,
      Buffer.from("\nendstream"),
    ]),
  ];
  const chunks = [Buffer.from("%PDF-1.4\n")];
  const offsets = [];
  let position = chunks[0].length;
  objects.forEach((body, index) => {
    offsets.push(position);
    const chunk = Buffer.concat([
      Buffer.from(`${index + 1} 0 obj\n`),
      body,
      Buffer.from("\nendobj\n"),
    ]);
    chunks.push(chunk);
    position += chunk.length;
  });
  const xref = [`xref`, `0 ${objects.length + 1}`, `0000000000 65535 f `]
    .concat(offsets.map((off) => `${String(off).padStart(10, "0")} 00000 n `))
    .join("\n");
  chunks.push(
    Buffer.from(
      `${xref}\ntrailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\n` +
        `startxref\n${position}\n%%EOF\n`,
    ),
  );
  return Buffer.concat(chunks);
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
    const { readFileSync } = await import("node:fs");
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
  const slowDoc = gridPdf(64000); // parses in seconds, not before the deadline
  const pooled = new LiteParse({ ...CFG, poolSize: 1, parseTimeoutMs: 500 });
  try {
    await pooled.warmUp();
    await assert.rejects(pooled.parse(slowDoc), (e) => {
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
  writeFileSync(slowPath, gridPdf(64000));
  const pooled = new LiteParse({ ...CFG, poolSize: 1, parseTimeoutMs: 500 });
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
