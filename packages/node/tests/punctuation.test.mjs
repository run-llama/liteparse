import { test } from "node:test";
import assert from "node:assert/strict";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { LiteParse } from "@llamaindex/liteparse";

const fixture = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../../crates/liteparse/tests/fixtures/typographic-punctuation.pdf",
);

for (const [normalizePunctuation, expected] of [
  [undefined, "quote 'word' and \"word\"; range 1-2 - done"],
  [false, "quote ‘word’ and “word”; range 1–2 — done"],
]) {
  test(`native punctuation: normalizePunctuation=${normalizePunctuation}`, async () => {
    const parser = new LiteParse({
      ocrEnabled: false,
      quiet: true,
      normalizePunctuation,
    });
    assert.equal(parser.getConfig().normalizePunctuation, normalizePunctuation ?? true);
    const result = await parser.parse(fixture);
    assert.equal(result.pages[0].text.trim(), expected);
  });
}
