// CLI argument validation: numeric flags and `screenshot --target-pages`.
//
// These drive the built CLI (dist/cli.js) like cli-text.test.mjs does.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageDir, "..", "..");
const cliPath = join(packageDir, "dist", "cli.js");
const samplePdf = join(repoRoot, "demo", "docs", "apple-10k-2024.pdf");

function runCli(args) {
  // The timeout keeps a regression (flag accepted, document actually parsed)
  // from hanging the suite.
  return spawnSync(process.execPath, [cliPath, ...args], { encoding: "utf8", timeout: 30_000 });
}

function withTempDir(fn) {
  const dir = mkdtempSync(join(tmpdir(), "liteparse-cli-args-"));
  try {
    return fn(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const invalidTargetPages = [
  ["5-3", /invalid --target-pages: invalid range: 5-3/],
  ["1-", /invalid --target-pages: invalid page number/],
  ["abc", /invalid --target-pages: invalid page number: abc/],
  ["1,,2", /invalid --target-pages: invalid page number/],
  [
    "1-4294967295",
    /invalid --target-pages: too many target pages: 1-4294967295 exceeds the limit of 100000/,
  ],
];

for (const [spec, message] of invalidTargetPages) {
  test(`screenshot rejects --target-pages ${JSON.stringify(spec)}`, () => {
    withTempDir((dir) => {
      const outDir = join(dir, "shots");
      const result = runCli([
        "screenshot",
        samplePdf,
        "-o",
        outDir,
        "--target-pages",
        spec,
        "--quiet",
      ]);
      assert.equal(result.status, 1, result.stderr);
      assert.match(result.stderr, message);
      // Validation happens before anything is created or rendered.
      assert.equal(existsSync(outDir), false);
    });
  });
}

test("screenshot de-duplicates and sorts --target-pages", () => {
  withTempDir((dir) => {
    const outDir = join(dir, "shots");
    const result = runCli([
      "screenshot",
      samplePdf,
      "-o",
      outDir,
      "--target-pages",
      "3-4,2,3,2",
      "--dpi",
      "36",
    ]);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(readdirSync(outDir).sort(), ["page_2.png", "page_3.png", "page_4.png"]);
    const logged = result.stderr.match(/screenshot page \d+/g);
    assert.deepEqual(logged, ["screenshot page 2", "screenshot page 3", "screenshot page 4"]);
  });
});

const numericFlags = [
  ["parse", "--max-pages", ["abc", "0", "-1", "1.5", "3abc", "4294967296"]],
  ["parse", "--num-workers", ["abc", "0", "-1"]],
  ["parse", "--dpi", ["abc", "0", "-5", "Infinity"]],
  ["is-complex", "--max-pages", ["abc", "0"]],
  ["screenshot", "--dpi", ["abc", "0"]],
  ["batch-parse", "--max-pages", ["abc", "0"]],
  ["batch-parse", "--dpi", ["abc", "0"]],
  ["batch-parse", "--num-workers", ["abc", "0"]],
];

for (const [command, flag, values] of numericFlags) {
  for (const value of values) {
    test(`${command} rejects ${flag} ${value}`, () => {
      withTempDir((dir) => {
        // Point every command at throwaway paths so a regression (flag silently
        // accepted) cannot write into the working directory.
        const positional =
          command === "batch-parse"
            ? [dir, join(dir, "out")]
            : command === "screenshot"
              ? [samplePdf, "-o", join(dir, "shots"), "--target-pages", "1"]
              : [samplePdf, ...(command === "parse" ? ["--no-ocr"] : [])];
        // `--flag=value` so negative values are not read as a separate option.
        const result = runCli([command, ...positional, `${flag}=${value}`, "--quiet"]);
        assert.equal(result.status, 1, result.stderr);
        assert.match(
          result.stderr,
          new RegExp(`option '${flag} <\\w+>' argument '${value}' is invalid`)
        );
      });
    });
  }
}

test("numeric flags still accept valid values", () => {
  const result = runCli([
    "parse",
    samplePdf,
    "--no-ocr",
    "--quiet",
    "--max-pages",
    "2",
    "--dpi",
    "72.5",
    "--num-workers",
    "2",
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.match(/^--- Page \d+ ---$/gm).length, 2);
});
