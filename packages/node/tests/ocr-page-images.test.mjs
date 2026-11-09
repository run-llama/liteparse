import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { LiteParse } from "@llamaindex/liteparse";

function blankPdf() {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
    ...Array(3).fill("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] >>"),
  ];
  let pdf = "%PDF-1.7\n";
  const offsets = objects.map((object, index) => {
    const offset = Buffer.byteLength(pdf);
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return offset;
  });
  const xref = Buffer.byteLength(pdf);
  pdf += "xref\n0 6\n0000000000 65535 f \n";
  for (const offset of offsets) pdf += `${String(offset).padStart(10, "0")} 00000 n \n`;
  return Buffer.from(`${pdf}trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`);
}

test("OCR images retain source page keys through pool and batch parsing", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "liteparse-ocr-images-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const pdf = blankPdf();
  const screenshots = await new LiteParse({ ocrEnabled: false, dpi: 144, quiet: true }).screenshot(pdf, [3]);
  const path = join(dir, "page-three.png");
  writeFileSync(path, screenshots[0].imageBuffer);

  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = Buffer.concat(chunks);
    const png = body.indexOf(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
    if (png < 0) { response.writeHead(400).end(); return; }
    const width = body.readUInt32BE(png + 16);
    const scale = width / 72;
    response.setHeader("Content-Type", "application/json");
    response.end(JSON.stringify({ results: [{
      text: `IMAGE${width}`, bbox: [40, 20, 60, 30].map((v) => v * scale), confidence: 0.93,
    }] }));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => { server.closeAllConnections(); server.close(); });
  const config = {
    ocrEnabled: true, quiet: true, emitWordBoxes: true,
    ocrServerUrl: `http://127.0.0.1:${server.address().port}`,
    ocrPageImages: { 3: { path, dpi: 144 } },
  };
  for (const poolSize of [undefined, 1]) {
    // Zero rendering DPI makes a discarded render fail as well: this must
    // bypass rendering, not merely replace the raster after creating it.
    const parser = new LiteParse({ ...config, targetPages: "3", dpi: 0, poolSize });
    try {
      const result = await parser.parse(pdf);
      assert.equal(result.pages[0].pageNum, 3);
      const item = result.pages[0].textItems.find((item) => item.text === "IMAGE144");
      assert.ok(item, result.text);
      assert.equal(item.x, 40);
      assert.equal(item.words[0].width, 20);
    } finally { parser.close(); }
  }
  const parser = new LiteParse({ ...config, dpi: 72 });
  const texts = [];
  for await (const batch of parser.parseBatches(pdf, { batchSize: 1 })) texts.push(batch.result.text.trim());
  assert.deepEqual(texts, ["IMAGE72", "IMAGE72", "IMAGE144"]);
});
