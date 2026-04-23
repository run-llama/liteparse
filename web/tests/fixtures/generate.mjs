// One-shot script to generate the test fixtures.
// Run with: node web/tests/fixtures/generate.mjs
// Fixtures produced:
//   - sample-text.pdf:  plain-text PDF, known string "Hello from LiteParse"
//   - sample-scanned.pdf: image-only PDF with no text layer, image contains
//     visible text "OCR TEST PAGE" rendered as black pixel-blocks so Tesseract
//     can recognize it.
//   - corrupt.pdf: %PDF-1.7 header followed by garbage bytes.

import { PDFDocument, rgb, StandardFonts } from "pdf-lib";
import { writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

async function makeTextPdf() {
  const doc = await PDFDocument.create();
  const page = doc.addPage([400, 200]);
  const font = await doc.embedFont(StandardFonts.Helvetica);
  page.drawText("Hello from LiteParse", {
    x: 50,
    y: 130,
    size: 24,
    font,
    color: rgb(0, 0, 0),
  });
  page.drawText("Browser text extraction fixture.", {
    x: 50,
    y: 90,
    size: 14,
    font,
    color: rgb(0, 0, 0),
  });
  return doc.save();
}

// Draw a very chunky bitmap of "OCR TEST PAGE" into an RGBA array we can embed
// as a PNG. Each glyph is a 5x7 matrix at ~20px per cell so Tesseract has
// plenty of pixels to work with.
function makeOcrImage() {
  // prettier-ignore
  const GLYPHS = {
    " ": ["     ","     ","     ","     ","     ","     ","     "],
    "O": [" ### ","#   #","#   #","#   #","#   #","#   #"," ### "],
    "C": [" ####","#    ","#    ","#    ","#    ","#    "," ####"],
    "R": ["#### ","#   #","#   #","#### ","# #  ","#  # ","#   #"],
    "T": ["#####","  #  ","  #  ","  #  ","  #  ","  #  ","  #  "],
    "E": ["#####","#    ","#    ","#####","#    ","#    ","#####"],
    "S": [" ####","#    ","#    "," ### ","    #","    #","#### "],
    "P": ["#### ","#   #","#   #","#### ","#    ","#    ","#    "],
    "A": [" ### ","#   #","#   #","#####","#   #","#   #","#   #"],
    "G": [" ####","#    ","#    ","#  ##","#   #","#   #"," ### "],
  };
  const text = "OCR TEST PAGE";
  const cell = 12; // pixels per glyph cell
  const glyphW = 5 * cell;
  const glyphH = 7 * cell;
  const spacing = cell;
  const padding = cell * 2;
  const width = padding * 2 + text.length * glyphW + (text.length - 1) * spacing;
  const height = padding * 2 + glyphH;
  const rgba = new Uint8Array(width * height * 4);
  // White background
  rgba.fill(255);
  // Alpha = 255 everywhere
  for (let i = 3; i < rgba.length; i += 4) rgba[i] = 255;

  const putPixel = (x, y, black) => {
    const v = black ? 0 : 255;
    const i = (y * width + x) * 4;
    rgba[i] = v;
    rgba[i + 1] = v;
    rgba[i + 2] = v;
  };

  let cursorX = padding;
  for (const ch of text) {
    const glyph = GLYPHS[ch] ?? GLYPHS[" "];
    for (let row = 0; row < 7; row++) {
      for (let col = 0; col < 5; col++) {
        const filled = glyph[row][col] === "#";
        if (!filled) continue;
        for (let dy = 0; dy < cell; dy++) {
          for (let dx = 0; dx < cell; dx++) {
            putPixel(cursorX + col * cell + dx, padding + row * cell + dy, true);
          }
        }
      }
    }
    cursorX += glyphW + spacing;
  }

  return { width, height, rgba };
}

// Minimal PNG encoder: converts raw RGBA to a PNG byte stream using DEFLATE.
// Uses Node's zlib for deflate. Single IDAT chunk, no filter bytes per line
// other than filter=0 (None). Produces valid PNG that pdf-lib accepts.
async function encodePng(width, height, rgba) {
  const { deflateSync } = await import("node:zlib");
  // Add filter byte (0 = None) at start of each scanline
  const stride = width * 4;
  const raw = new Uint8Array((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    raw.set(rgba.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
  }
  const compressed = deflateSync(Buffer.from(raw));

  const crcTable = (() => {
    const t = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      t[n] = c >>> 0;
    }
    return t;
  })();
  const crc32 = (buf) => {
    let c = 0xffffffff;
    for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };

  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length, 0);
    const typeBuf = Buffer.from(type, "ascii");
    const crcBuf = Buffer.alloc(4);
    crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
    return Buffer.concat([len, typeBuf, data, crcBuf]);
  };

  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  ihdr[10] = 0; // compression
  ihdr[11] = 0; // filter
  ihdr[12] = 0; // interlace
  return Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", compressed), chunk("IEND", Buffer.alloc(0))]);
}

async function makeScannedPdf() {
  const { width, height, rgba } = makeOcrImage();
  const png = await encodePng(width, height, rgba);
  const doc = await PDFDocument.create();
  const pageWidth = width;
  const pageHeight = height;
  const page = doc.addPage([pageWidth, pageHeight]);
  const image = await doc.embedPng(png);
  page.drawImage(image, { x: 0, y: 0, width: pageWidth, height: pageHeight });
  return doc.save();
}

async function main() {
  const textPdf = await makeTextPdf();
  writeFileSync(resolve(HERE, "sample-text.pdf"), textPdf);
  console.log("wrote sample-text.pdf", textPdf.length, "bytes");

  const scannedPdf = await makeScannedPdf();
  writeFileSync(resolve(HERE, "sample-scanned.pdf"), scannedPdf);
  console.log("wrote sample-scanned.pdf", scannedPdf.length, "bytes");

  // Corrupt PDF: valid header, garbage payload
  const corrupt = Buffer.concat([Buffer.from("%PDF-1.7\n"), Buffer.from("this is not a real PDF at all")]);
  writeFileSync(resolve(HERE, "corrupt.pdf"), corrupt);
  console.log("wrote corrupt.pdf", corrupt.length, "bytes");

  // Non-PDF file
  writeFileSync(resolve(HERE, "not-a-pdf.txt"), "I am a plain text file\n");
  console.log("wrote not-a-pdf.txt");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
