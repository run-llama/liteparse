import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { LiteParse } from "../dist/lib.js";

const fixture = fileURLToPath(
  new URL("../../../integration_tests_data/macos_ocr.png", import.meta.url)
);
const config = {
  ocrEnabled: true,
  ocrBackend: "macos-vision",
  macOcrLanguages: ["en-US"],
  macOcrRecognition: "accurate",
  emitWordBoxes: true,
  outputFormat: "text",
  quiet: true,
};

for (const poolSize of [undefined, 2]) {
  test(
    `real Vision recognition through Node (poolSize=${poolSize})`,
    { skip: process.platform !== "darwin" },
    async () => {
      const parser = new LiteParse({ ...config, poolSize });
      try {
        const resolved = parser.getConfig();
        assert.equal(resolved.ocrBackend, "macos-vision");
        assert.deepEqual(resolved.macOcrLanguages, ["en-US"]);
        assert.equal(resolved.macOcrRecognition, "accurate");
        const result = await parser.parse(fixture);
        assert.match(result.text, /Invoice 12345/);
        const words = result.pages[0].textItems.flatMap((item) => item.words);
        const invoice = words.find((word) => word.text === "Invoice");
        assert.ok(invoice);
        assert.ok(Math.abs(invoice.x - 38.4) < 10);
        assert.ok(Math.abs(invoice.y - 139.2) < 15);
      } finally {
        parser.close();
      }
    }
  );
}

test("Node CLI selects real Vision OCR", { skip: process.platform !== "darwin" }, () => {
  const cli = fileURLToPath(new URL("../dist/cli.js", import.meta.url));
  const result = spawnSync(
    process.execPath,
    [
      cli,
      "parse",
      fixture,
      "--ocr-backend",
      "macos-vision",
      "--mac-ocr-language",
      "en-US",
      "--mac-ocr-recognition",
      "fast",
      "--format",
      "text",
      "--quiet",
    ],
    { encoding: "utf8", timeout: 60_000 }
  );
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Invoice 12345/);
});

test("native config rejects unknown backend and recognition level", () => {
  assert.throws(() => new LiteParse({ ocrBackend: "invalid" }), /invalid OCR backend/);
  assert.throws(
    () => new LiteParse({ macOcrRecognition: "invalid" }),
    /invalid Mac OCR recognition/
  );
});

test(
  "Vision reports its platform requirement",
  { skip: process.platform === "darwin" },
  async () => {
    await assert.rejects(new LiteParse(config).parse(fixture), /requires macOS/);
  }
);
