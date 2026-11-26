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

import { LiteParse } from "../dist/lib.js";

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageDir, "..", "..");
const cliPath = join(packageDir, "dist", "cli.js");
const samplePdf = join(repoRoot, "demo", "docs", "apple-10k-2024.pdf");

function runCliJson(args) {
  const stdout = execFileSync(process.execPath, [cliPath, ...args], {
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
  });
  return JSON.parse(stdout);
}

test("parse --format json omits blocks unless --extract-blocks is set", () => {
  const json = runCliJson([
    "parse",
    samplePdf,
    "--no-ocr",
    "--max-pages",
    "1",
    "--format",
    "json",
    "--quiet",
  ]);

  assert.equal(json.pages.length, 1);
  assert.equal("blocks" in json.pages[0], false);
});

test("parse --format json --extract-blocks emits Rust-schema layout blocks", async () => {
  const api = await new LiteParse({
    ocrEnabled: false,
    maxPages: 3,
    extractBlocks: true,
    quiet: true,
  }).parse(samplePdf);

  const cli = runCliJson([
    "parse",
    samplePdf,
    "--no-ocr",
    "--max-pages",
    "3",
    "--format",
    "json",
    "--extract-blocks",
    "--quiet",
  ]);

  assert.equal(cli.pages.length, api.pages.length);

  let sawBold = false;
  let sawFigure = false;
  let sawTableWithHeader = false;

  for (let i = 0; i < api.pages.length; i++) {
    const apiBlocks = api.pages[i].blocks;
    const cliBlocks = cli.pages[i].blocks;
    assert.ok(Array.isArray(cliBlocks), `page ${i + 1} should include blocks`);
    assert.equal(cliBlocks.length, apiBlocks.length);

    for (let j = 0; j < apiBlocks.length; j++) {
      const apiBlock = apiBlocks[j];
      const cliBlock = cliBlocks[j];

      assert.equal(cliBlock.kind, apiBlock.kind);
      assert.equal(cliBlock.text, apiBlock.text);
      assert.equal(cliBlock.level, apiBlock.level);
      assert.equal(cliBlock.ordered, apiBlock.ordered);
      assert.equal(cliBlock.marker, apiBlock.marker);
      assert.deepEqual(cliBlock.lines, apiBlock.lines);
      assert.equal(cliBlock.lang, apiBlock.lang);
      assert.equal(cliBlock.id, apiBlock.id);
      assert.equal(cliBlock.format, apiBlock.format);
      assert.deepEqual(cliBlock.text_item_indices, apiBlock.textItemIndices);
      assert.equal("textItemIndices" in cliBlock, false);

      if (apiBlock.kind === "figure") sawFigure = true;

      // Matches Rust's `#[serde(skip_serializing_if = "std::ops::Not::not")]`.
      if (apiBlock.bold) {
        assert.equal(cliBlock.bold, true);
        sawBold = true;
      } else {
        assert.equal("bold" in cliBlock, false);
      }
      if (apiBlock.italic) {
        assert.equal(cliBlock.italic, true);
      } else {
        assert.equal("italic" in cliBlock, false);
      }

      assert.deepEqual(cliBlock.bbox, apiBlock.bbox);

      if (apiBlock.header !== undefined) {
        sawTableWithHeader = true;
        assert.equal(cliBlock.header.length, apiBlock.header.length);
        for (let c = 0; c < apiBlock.header.length; c++) {
          const apiCell = apiBlock.header[c];
          const cliCell = cliBlock.header[c];
          assert.equal(cliCell.text, apiCell.text);
          assert.deepEqual(cliCell.bbox, apiCell.bbox);
          assert.deepEqual(cliCell.text_item_indices, apiCell.textItemIndices);
          assert.equal("textItemIndices" in cliCell, false);
        }
      }

      if (apiBlock.rows !== undefined) {
        assert.equal(cliBlock.rows.length, apiBlock.rows.length);
        for (let r = 0; r < apiBlock.rows.length; r++) {
          assert.equal(cliBlock.rows[r].length, apiBlock.rows[r].length);
          for (let c = 0; c < apiBlock.rows[r].length; c++) {
            const apiCell = apiBlock.rows[r][c];
            const cliCell = cliBlock.rows[r][c];
            assert.equal(cliCell.text, apiCell.text);
            assert.deepEqual(cliCell.bbox, apiCell.bbox);
            assert.deepEqual(cliCell.text_item_indices, apiCell.textItemIndices);
            assert.equal("textItemIndices" in cliCell, false);
          }
        }
      }
    }
  }

  assert.ok(sawBold, "fixture should exercise bold: true serialization");
  assert.ok(sawFigure, "fixture should exercise figure block serialization");
  assert.ok(sawTableWithHeader, "fixture should exercise table header/rows cell serialization");
});

test("batch-parse --format json --extract-blocks includes blocks in output files", () => {
  const testDir = mkdtempSync(join(tmpdir(), "liteparse-cli-json-"));
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
        "1",
        "--format",
        "json",
        "--extract-blocks",
        "--quiet",
      ],
      { encoding: "utf8" },
    );
    const json = JSON.parse(readFileSync(join(outputDir, "sample.json"), "utf8"));
    assert.equal(json.pages.length, 1);
    assert.ok(Array.isArray(json.pages[0].blocks));
    assert.ok(json.pages[0].blocks.length > 0);
    assert.equal("textItemIndices" in json.pages[0].blocks[0], false);
    assert.ok(Array.isArray(json.pages[0].blocks[0].text_item_indices));
  } finally {
    rmSync(testDir, { recursive: true, force: true });
  }
});
