import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { LiteParse } from "@llamaindex/liteparse";

// Synthetic solid JPEG and RGB PNG; the PNG includes a text chunk so
// decoding and re-encoding cannot accidentally pass the byte-preservation check.
const jpeg = Buffer.from("/9j/4AAQSkZJRgABAgAAAQABAAD/wAARCABIACQDAREAAhEBAxEB/9sAQwADAgIDAgIDAwMDBAMDBAUIBQUEBAUKBwcGCAwKDAwLCgsLDQ4SEA0OEQ4LCxAWEBETFBUVFQwPFxgWFBgSFBUU/9sAQwEDBAQFBAUJBQUJFA0LDRQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD4MoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgD//2Q==", "base64");
const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAEgAAACQCAIAAAAk4rpSAAACFElEQVR4Ae3AA6AkWZbG8f937o3IzKdyS2Oubdu2bdu2bdu2bWmMnpZKr54yMyLu+Xa3anqmhztr1a/+3d/9Hf8HEfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9NBP83EfzfRPB/E8H/TQT/NxH830TwfxPB/00E/zcR/N9E8H8Twf9N/CNzqgOcZbaKUAAAABl0RVh0Q29tbWVudABieXRlLXByZXNlcnZhdGlvbu3CW/kAAAAASUVORK5CYII=", "base64");

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

test("original JPEG/PNG uploads survive bindings, pools and batch page numbering", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "liteparse-ocr-images-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const pdf = blankPdf();
  const jpegPath = join(dir, "one.bin");
  const pngPath = join(dir, "three.png");
  writeFileSync(jpegPath, jpeg);
  writeFileSync(pngPath, png);
  const uploads = [];
  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = Buffer.concat(chunks);
    const start = body.indexOf(Buffer.from("\r\n\r\n")) + 4;
    const boundary = request.headers["content-type"].split("boundary=")[1];
    const end = body.indexOf(Buffer.from(`\r\n--${boundary}`), start);
    const data = body.subarray(start, end);
    const headers = body.subarray(0, start).toString();
    let text, dpi, mime, file;
    if (data.equals(jpeg)) {
      [text, dpi, mime, file] = ["JPEG", 36, "image/jpeg", "image.jpg"];
    } else if (data.equals(png)) {
      [text, dpi, mime, file] = ["PNG", 72, "image/png", "image.png"];
    } else if (data.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])) && data.readUInt32BE(16) === 72) {
      [text, dpi, mime, file] = ["RENDERED", 72, "image/png", "image.png"];
    } else { response.writeHead(400).end(); return; }
    uploads.push(headers.includes(mime) && headers.includes(`filename="${file}"`));
    response.setHeader("Content-Type", "application/json");
    response.end(JSON.stringify({ results: [{ text, bbox: [40, 20, 60, 30].map((v) => v * dpi / 72), confidence: 0.93 }] }));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => { server.closeAllConnections(); server.close(); });
  const config = {
    ocrEnabled: true, quiet: true, emitWordBoxes: true,
    ocrServerUrl: `http://127.0.0.1:${server.address().port}`,
    ocrPageImages: { 1: jpegPath, 3: pngPath },
  };
  for (const poolSize of [undefined, 1]) {
    // A discarded render at zero DPI would fail too.
    const parser = new LiteParse({ ...config, targetPages: "1,3", dpi: 0, poolSize });
    try {
      const result = await parser.parse(pdf);
      assert.deepEqual(result.pages.map((page) => page.pageNum), [1, 3]);
      for (const [index, text] of ["JPEG", "PNG"].entries()) {
        const item = result.pages[index].textItems.find((item) => item.text === text);
        assert.ok(item, result.text);
        assert.equal(item.x, 40);
        assert.equal(item.words[0].width, 20);
      }
    } finally { parser.close(); }
  }
  const parser = new LiteParse({ ...config, dpi: 72 });
  const texts = [];
  for await (const batch of parser.parseBatches(pdf, { batchSize: 1 })) texts.push(batch.result.text.trim());
  assert.deepEqual(texts, ["JPEG", "RENDERED", "PNG"]);
  assert.ok(uploads.every(Boolean), "uploaded MIME type or filename changed");
});
