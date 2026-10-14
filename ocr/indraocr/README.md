# Indra OCR Service

A FastAPI server wrapping [Indra OCR](https://huggingface.co/datalab-to/indra-ocr)
to conform to the LiteParse OCR API specification (see `../../OCR_API_SPEC.md`).

Indra is a small CPU-first OCR model (line detection + CTC recognition, ~18 MB of
ONNX weights in total). It runs on ONNX Runtime — no GPU, no torch, and no
separate inference server — and straightens skewed or sideways pages before
reading them.

> **License — read before deploying.** The `indra-ocr` code is Apache 2.0, but the
> model weights use a **modified OpenRAIL-M license**: free for research, personal
> use, and startups under $20M in funding/revenue, and not usable competitively
> with Datalab's API. Broader commercial use needs a license from Datalab — see
> the [model card](https://huggingface.co/datalab-to/indra-ocr) for the exact
> terms. This server downloads those weights on first run.

## Build and Run

```bash
# install and run (in one command)
uv run server.py
```

The first run downloads the model weights (~18 MB) from Hugging Face; they are
cached afterward.

## Docker

```bash
# Build (bakes the weights into the image)
docker build -t indraocr-liteparse .

# Run
docker run --rm -p 8831:8831 indraocr-liteparse
```

The image needs no network access at runtime.

Verify it is up:

```bash
curl http://localhost:8831/health
curl -X POST -F "file=@image.png" http://localhost:8831/ocr
```

## Usage

The service exposes:

- `POST /ocr` — Perform OCR on an uploaded image
- `GET /health` — Health check

### Parameters

- `file` — Image file (multipart/form-data)
- `language` — Language code (accepted for API compatibility; **ignored**, since
  one model reads every supported script)

### Example

```bash
curl -X POST -F "file=@image.png" http://localhost:8831/ocr
```

### Response Format

```json
{
  "results": [
    {
      "text": "recognized text",
      "bbox": [x1, y1, x2, y2],
      "confidence": 0.95,
      "polygon": [[x1, y1], [x2, y2], [x3, y3], [x4, y4]]
    }
  ]
}
```

Results are **line-level**. Boxes are in the coordinates of the uploaded image;
when Indra turns a sideways page upright, `polygon` is given in the upright
reading frame (as the spec describes) so LiteParse can recover the rotation.

## Supported Languages

Per the model card, Indra OCRs Latin scripts (~50 languages), Cyrillic, Greek,
Devanagari, Bengali, and Chinese (simplified and traditional). **Japanese,
Korean, and Arabic are not supported for OCR** — use PaddleOCR or Surya for
those scripts.

## Performance

`INDRA_WORKERS` sets how many recognizers serve requests in parallel (default:
`min(4, cores)`), and `INDRA_THREADS` the ONNX Runtime threads of each (default
`1`). LiteParse OCRs several pages at once (`--num-workers`), so a pool of
single-threaded recognizers keeps every page busy without oversubscribing the CPU.

```bash
INDRA_WORKERS=8 INDRA_THREADS=1 uv run server.py
```

## Use with LiteParse

```bash
lit parse document.pdf --ocr-server-url http://localhost:8831/ocr
```

Or in code:

```typescript
import { LiteParse } from 'liteparse';

const parser = new LiteParse({
  ocrServerUrl: 'http://localhost:8831/ocr',
});

const result = await parser.parse('document.pdf');
```

## Testing

```bash
uv run pytest test_server.py
```

Tests mock the Indra recognizer, so they run without downloading any models.

## Notes

- Default port is 8831 (easyocr 8828, paddleocr 8829, suryaocr 8830).
- CPU only; no GPU or external inference backend is needed.
