---
title: OCR Configuration
description: Configure OCR in LiteParse — Tesseract, native Apple Vision, or HTTP servers.
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

## Native Apple Vision (macOS)

On macOS 10.15 or later, select `macos-vision` to use the system's Vision
framework. Recognition runs locally without a Python runtime, OCR server,
or Tesseract language-data download. Both Intel and Apple Silicon are supported.

```bash
lit parse scan.pdf --ocr-backend macos-vision
lit parse scan.pdf --ocr-backend macos-vision \
  --mac-ocr-language en-US --mac-ocr-language fr-FR \
  --mac-ocr-recognition accurate
```

```python
from liteparse import LiteParse

parser = LiteParse(
    ocr_enabled=True,
    ocr_backend="macos-vision",
    mac_ocr_languages=["en-US", "fr-FR"],
    mac_ocr_recognition="accurate",
)
result = parser.parse("scan.pdf")
```

```typescript
import { LiteParse } from "@llamaindex/liteparse";

const parser = new LiteParse({
  ocrEnabled: true,
  ocrBackend: "macos-vision",
  macOcrLanguages: ["en-US", "fr-FR"],
  macOcrRecognition: "accurate",
});
const result = await parser.parse("scan.pdf");
```

Rust callers can set `LiteParseConfig::ocr_backend` to
`OcrBackend::MacosVision`, with `mac_ocr_languages` and `mac_ocr_recognition`.
This backend also works with `--no-default-features`; set `ocr_enabled: true`
when using that build because OCR defaults to disabled without Tesseract.

Vision accepts language tags such as `en-US` and `zh-Hans`. These are separate
from `ocr_language` / `--ocr-language`, which uses Tesseract language codes.
The language list defaults to empty: Vision detects languages automatically
on macOS 13+ and uses its default languages on older systems. Explicit tags
are checked against the running OS. An unsupported tag returns an error
listing the available languages.

Recognition defaults to `accurate`; `fast` trades accuracy for speed and may
support fewer languages. Availability depends on the macOS version and level.
Vision word boxes are converted to the same page coordinates as other engines,
including when `emit_word_boxes` / `emitWordBoxes` is enabled.

The default backend is `auto`, which selects HTTP when `ocr_server_url` is set
and otherwise uses Tesseract. Selecting `macos-vision` on another platform
(including WASM) returns a platform error when OCR is enabled. Explicit
`tesseract` or `macos-vision` selection cannot be combined with an OCR server
URL. Caller-supplied OCR engines keep precedence over built-in selection.

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
