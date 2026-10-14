# ocr/

Example OCR server implementations that conform to the LiteParse OCR API specification.

These servers allow you to use alternative OCR engines instead of the built-in Tesseract.js.

## Why Use an External OCR Server?

| Feature | Tesseract.js (built-in) | EasyOCR | PaddleOCR | Surya | Indra |
|---------|-------------------------|---------|-----------|-------|-------|
| Setup | Zero (included) | uv | uv | uv | uv |
| Speed | Moderate | Moderate | Fast (2-3x) | Moderate (GPU recommended) | Fast (CPU only) |
| Accuracy (Latin) | Good | Good | Good | Excellent | Very good |
| Accuracy (CJK) | Fair | Good | Excellent | Excellent | Good (Chinese only) |
| Languages | 100+ | 80+ | 80+ | 91 | ~55 (Latin, Cyrillic, Greek, Devanagari, Bengali, Chinese) |
| Memory | In-process | Separate | Separate | Separate | Separate |

**Recommendations:**
- **Quick start**: Use built-in Tesseract (no setup)
- **Asian languages**: Use PaddleOCR (best CJK support)
- **Multilingual / broad scripts**: Use Surya (strong multilingual accuracy)
- **General use**: EasyOCR (good balance)
- **Fast CPU OCR / scans**: Use Indra (no GPU; check the model-weights license)

## Available Servers

### [easyocr/](./easyocr/)
Flask server wrapping EasyOCR library.
- Port: **8828**
- Good general-purpose OCR
- 80+ languages

### [paddleocr/](./paddleocr/)
Flask server wrapping PaddleOCR library.
- Port: **8829**
- Excellent for Chinese, Japanese, Korean
- 2-3x faster than EasyOCR

### [suryaocr/](./suryaocr/)
FastAPI server wrapping Surya OCR 2.
- Port: **8830**
- Multilingual foundation model (90+ languages)
- Block-level output; GPU recommended

### [indraocr/](./indraocr/)
FastAPI server wrapping Indra OCR.
- Port: **8831**
- Small CPU-only model (ONNX Runtime, no GPU)
- Line-level output; straightens skewed and sideways pages
- Model weights are under a modified OpenRAIL-M license (see its README)

## Quick Start

```bash
# Start EasyOCR server
cd ocr/easyocr
uv run server.py

# OR start PaddleOCR server
cd ocr/paddleocr
uv run server.py

# OR start Surya OCR server
cd ocr/suryaocr
uv run server.py

# OR start Indra OCR server
cd ocr/indraocr
uv run server.py
```

Then use with LiteParse:

```bash
# CLI
lit parse document.pdf --ocr-server-url http://localhost:8828/ocr

# Code
const parser = new LiteParse({
  ocrServerUrl: 'http://localhost:8828/ocr',
  ocrLanguage: 'en',
});
```

## API Specification

All servers implement the same API (defined in `OCR_API_SPEC.md`):

**Endpoint:** `POST /ocr`

**Request:**
- Content-Type: `multipart/form-data`
- Fields:
  - `file` - Image file
  - `language` - Language code (e.g., 'en', 'zh', 'ja')

**Response:**
```json
{
  "results": [
    {
      "text": "recognized text",
      "bbox": [x1, y1, x2, y2],
      "confidence": 0.95
    }
  ]
}
```

## Creating a Custom OCR Server

To implement your own OCR server:

1. Create a Flask/FastAPI/Express server
2. Accept `POST /ocr` with multipart form data
3. Return JSON with `results` array containing:
   - `text` - Recognized text string
   - `bbox` - Bounding box as `[x1, y1, x2, y2]`
   - `confidence` - Confidence score (0-1)

4. (Optional) Implement `GET /health` endpoint

See the existing servers as reference implementations.

## Language Codes

Most servers accept ISO 639-1 codes (e.g., 'en', 'zh', 'ja') and map them internally:

| ISO Code | Language | Notes |
|----------|----------|-------|
| en | English | |
| zh | Chinese (Simplified) | |
| zh-tw | Chinese (Traditional) | |
| ja | Japanese | |
| ko | Korean | |
| fr | French | |
| de | German | |
| es | Spanish | |
| ar | Arabic | |
| hi | Hindi | |
