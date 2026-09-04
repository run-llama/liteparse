import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageDir, "..", "..");
const cliPath = join(packageDir, "dist", "cli.js");
const samplePdf = join(repoRoot, "demo", "docs", "apple-10k-2024.pdf");

function pageHeaders(output) {
  return output.match(/^--- Page \d+ ---$/gm) ?? [];
}

test("parse preserves page boundaries in text output", () => {
  const output = execFileSync(
    process.execPath,
    [
      cliPath,
      "parse",
      samplePdf,
      "--no-ocr",
      "--max-pages",
      "2",
      "--format",
      "text",
      "--quiet",
    ],
    { encoding: "utf8" },
  );

  assert.deepEqual(pageHeaders(output), ["--- Page 1 ---", "--- Page 2 ---"]);
});

test("batch-parse preserves page boundaries in text output", () => {
  const testDir = mkdtempSync(join(tmpdir(), "liteparse-cli-text-"));
  const inputDir = join(testDir, "input");
  const outputDir = join(testDir, "output");
  mkdirSync(inputDir);
  mkdirSync(outputDir);
  copyFileSync(samplePdf, join(inputDir, "sample.pdf"));

  try {
    execFileSync(
      process.execPath,
      [
        cliPath,
        "batch-parse",
        inputDir,
        outputDir,
        "--no-ocr",
        "--max-pages",
        "2",
        "--format",
        "text",
        "--quiet",
      ],
      { encoding: "utf8" },
    );
    const output = readFileSync(join(outputDir, "sample.txt"), "utf8");
    assert.deepEqual(pageHeaders(output), [
      "--- Page 1 ---",
      "--- Page 2 ---",
    ]);
  } finally {
    rmSync(testDir, { recursive: true, force: true });
  }
});
