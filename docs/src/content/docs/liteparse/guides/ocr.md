---
title: OCR Configuration
description: Configure OCR in LiteParse — built-in Tesseract, or bring your own via HTTP servers.
sidebar:
  order: 5
---

LiteParse uses OCR selectively — only on embedded images or pages where native text extraction didn't find text. This keeps parsing fast while still capturing text from scanned pages and embedded images.

## Built-in Tesseract (default)

Tesseract is bundled with LiteParse and works out of the box. Just run:

```bash
lit parse document.pdf
```

### Language support

Specify the OCR language for better accuracy on non-English documents:

```bash
lit parse document.pdf --ocr-language fra    # French
lit parse document.pdf --ocr-language deu    # German
lit parse document.pdf --ocr-language jpn    # Japanese
```

Tesseract uses [ISO 639-3](https://tesseract-ocr.github.io/tessdoc/Data-Files-in-different-versions.html) language codes (`eng`, `fra`, `deu`, etc.).

### Offline / air-gapped environments

For environments without internet access, point Tesseract at a local directory containing pre-downloaded `.traineddata` files:

```bash
# Via environment variable
export TESSDATA_PREFIX=/path/to/tessdata
lit parse document.pdf --ocr-language eng

# Or via CLI flag
lit parse document.pdf --tessdata-path /path/to/tessdata
```

The `tessdata_path` / `tessdataPath` option is also available in the library APIs.

### Troubleshooting: missing language data

If Tesseract can't find its language data, you'll see an error like:

```
Error opening data file tessdata/eng.traineddata
Please make sure the TESSDATA_PREFIX environment variable is set to your "tessdata" directory.
Failed loading language 'eng'
```

The bundled Tesseract still needs the `.traineddata` file for your language. It is normally downloaded and cached automatically on first use (under `~/.tesseract-rs/tessdata` on Linux, `~/Library/Application Support/tesseract-rs/tessdata` on macOS); if that download didn't happen — e.g. offline, restricted network, or a sandboxed install — OCR cannot run.

To resolve it, do any one of the following:

- Download the language file (e.g. [`eng.traineddata`](https://github.com/tesseract-ocr/tessdata)) and point LiteParse at it with `TESSDATA_PREFIX` or `--tessdata-path` (see [above](#offline--air-gapped-environments)).
- Use an [HTTP OCR server](#http-ocr-servers) instead of the built-in engine.
- Disable OCR with `--no-ocr` if you don't need it.

When language data is missing for **every** page, LiteParse now fails with a clear `OCR failed for all N page(s)` error instead of returning a document with silently-empty OCR text — so an unresolved setup surfaces immediately rather than producing partial results that look complete.

### Disabling OCR

If you don't need OCR (pure native-text PDFs, or you don't care about images), disable it for faster parsing:

```bash
lit parse document.pdf --no-ocr
```

## HTTP OCR servers

For higher accuracy or GPU-accelerated OCR, you can point LiteParse at an HTTP OCR server. LiteParse ships with ready-to-use examples for popular OCR engines.

### EasyOCR

```bash
# Start the EasyOCR server (requires Python)
git clone https://github.com/run-llama/liteparse.git
cd liteparse/ocr/easyocr
pip install -r requirements.txt
python server.py

# Parse with EasyOCR in another terminal
lit parse document.pdf --ocr-server-url http://localhost:8828/ocr
```

### PaddleOCR

```bash
# Start the PaddleOCR server (requires Python)
git clone https://github.com/run-llama/liteparse.git
cd liteparse/ocr/paddleocr
pip install -r requirements.txt
python server.py

# Parse with PaddleOCR in another terminal
lit parse document.pdf --ocr-server-url http://localhost:8828/ocr
```

### Authenticated servers

Pass extra HTTP headers on every OCR request with `--ocr-server-header`, repeating the flag for each header:

```bash
lit parse document.pdf \
  --ocr-server-url https://ocr.internal/ocr \
  --ocr-server-header "Authorization: Bearer $TOKEN" \
  --ocr-server-header "X-Tenant: acme"
```

In the libraries this is `ocr_server_headers` / `ocrServerHeaders`, taking a map of header name to value. It has no effect unless an OCR server URL is set.

### Parallel OCR workers

LiteParse OCRs multiple pages in parallel. By default, it uses one fewer worker than your CPU core count. Override this with:

```bash
lit parse document.pdf --num-workers 8
```

This is useful if you need to slow down OCR requests to an external server or if your OCR engine is GPU-accelerated and can handle more concurrency.

## Custom OCR servers

You can integrate any OCR engine by implementing the LiteParse OCR API. Your server needs a single endpoint:

```
POST /ocr
Content-Type: multipart/form-data
```

**Request fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `file` | binary | Yes | Image file (PNG, JPG, etc.) |
| `language` | string | No | ISO 639-1 language code (default: `en`) |

> **Watch the language code.** LiteParse forwards whatever `--ocr-language` is set to, which defaults to the Tesseract-style ISO 639-3 code `eng` — not `en`. Your server should accept both. The bundled EasyOCR and PaddleOCR servers do this by mapping `eng` → `en` before dispatch; do the same in a custom server, or pass `--ocr-language en` explicitly.

**Response format:**

```json
{
  "results": [
    {
      "text": "recognized text",
      "bbox": [12, 40, 220, 62],
      "confidence": 0.95
    }
  ]
}
```

Each result contains:

| Field | Type | Description |
|-------|------|-------------|
| `text` | string | Recognized text |
| `bbox` | `[x1, y1, x2, y2]` | Bounding box in pixels. Origin is top-left, x goes right, y goes down |
| `confidence` | number | Score from 0.0 to 1.0 |

### Testing your server

```bash
# Quick test with curl
curl -X POST http://localhost:8828/ocr \
  -F "file=@test.png" \
  -F "language=en" | jq .

# Use with LiteParse
lit parse document.pdf --ocr-server-url http://localhost:8828/ocr
```

### Common Gotchas

- Return `{"results": []}` if no text is detected
- Bounding boxes must be axis-aligned (`[x1, y1, x2, y2]` where top-left to bottom-right)
- If your engine returns rotated boxes, convert to axis-aligned by taking min/max coordinates
- If your engine doesn't provide confidence scores, return `1.0`
- Results should be in reading order (top-to-bottom, left-to-right)
- Cache OCR models in memory rather than reloading per request

## OCR in the browser (WASM)

The built-in Tesseract and HTTP OCR backends are not available in the WASM build. Instead, you can pass a custom `ocrEngine` object with a `recognize` method. See the [browser usage guide](/liteparse/guides/browser-usage/) for details.

### A note on OCR approaches

These days, its common to apply the term "OCR" to both traditional approaches and newer LLM-based document understanding models.

The LiteParse OCR API is designed specifically for approaches that return text with bounding boxes.

If you are trying to integrate a method that doesn't return bounding boxes, you will have to generate dummy bounding boxes.

## Reusing page images

If your application already has full-page screenshots, supply them as OCR
inputs to avoid rendering those pages again. LiteParse still reads the PDF for
native text and decides which pages need OCR. Pages without a supplied image
use the normal renderer.

```typescript
const parser = new LiteParse({
  ocrEnabled: true,
  ocrPageImages: {
    1: "/path/to/page-1.png",
    3: "/path/to/page-3.jpg",
  },
});
const result = await parser.parse("document.pdf");
```

Python accepts `ocr_page_images={1: "/path/to/page-1.png"}`.
Rust callers set `LiteParseConfig::ocr_page_images` to a `HashMap<u32, String>`
of page numbers to local paths. Composed Rust pipelines use the same map
in `stages::OcrRenderOptions::page_images`.

Keys always refer to **1-based source document pages**, including selected-page
and batch parses. LiteParse derives DPI from the image's pixel dimensions and
PDF page size, independently of image resolution metadata and the parser's
requested rendering DPI. Each image must cover the entire page in the
orientation produced by the PDF's `/Rotate` and any configured
`pageOrientationCorrections`. Its content, including form appearances, must
match the PDF. Cropped, deskewed and stretched images are unsupported.

With HTTP OCR, unmodified 8-bit JPEG/PNG images without an alpha channel are
uploaded as their original bytes, with the matching MIME type. LiteParse reads
the image header for dimensions; it does not decode and re-encode those pixels. The OCR server validates the compressed pixel data.
Images with alpha channels, higher bit depths, EXIF rotation or native text that
needs masking use decoded pixels. Pixel-based OCR engines, including Tesseract, keep their
normal decoded inputs.

Rust OCR engines receive a single `OcrImage` through `recognize_image`.
Existing engines can keep implementing the raw-pixel `recognize` method: the
default adapter supplies pixels in the engine's preferred RGB/grayscale format.
Engines that work with files can override `recognize_image` and borrow encoded
bytes through `image.encoded()`. Pixel edits consume the image with
`into_pixels`, then replace it with `OcrImage::Pixels`.

**Rust stage migration:** `OcrRaster::image` replaces its `pixels`, `width` and
`height` fields. Use `image.dimensions()` and `image.pixels(grayscale)` when
consuming stage outputs directly. Serialized stage payloads now contain a tagged
`image` object. The Node.js/Python result types and existing OCR engine methods
are unchanged.

LiteParse derives the scale from the longer page axis and checks the other
axis agrees within one pixel of raster rounding. An unreadable or incompatible
supplied image is a page error; it is not silently replaced with a render. `continueOnPageError` applies
as it does to rendering failures. Keep image files available until parsing
finishes, including when using worker pools or batch iterators. The parser
continues to apply its native-text masking and crop filters. Supplying OCR
images does not replace screenshots requested through `extractScreenshots`.

This file-path option is available in the Rust, Node.js and Python APIs.
