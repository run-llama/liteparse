# EasyOCR Service

This is a simple Flask server that wraps EasyOCR to conform to the LiteParse OCR API specification (see `../../OCR_API_SPEC.md`).

## Build and Run

```bash
# Build Docker image
docker build -t liteparse-easyocr .

# Run container
docker run -p 8828:8828 liteparse-easyocr
```

## Usage

The service exposes a single endpoint:

- `POST /ocr` - Perform OCR on an uploaded image

### Parameters

- `file` - Image file (multipart/form-data)
- `language` - Language code (e.g., 'en', 'fr', 'de')

### Example

```bash
curl -X POST -F "file=@image.png" -F "language=en" http://localhost:8828/ocr
```

### Response Format

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

This conforms to the LiteParse OCR API specification.

## Supported Languages

EasyOCR supports 80+ languages. Common language codes:

- `en` - English
- `fr` - French
- `de` - German
- `es` - Spanish
- `zh` - Chinese
- `ja` - Japanese
- `ko` - Korean
- `ar` - Arabic

Full list: https://www.jaided.ai/easyocr/

## Use with LiteParse

Once the server is running, use it with LiteParse OSS:

```bash
# Parse with EasyOCR
pnpm parse document.pdf --ocr-server-url http://localhost:8828/ocr

# With specific language
pnpm parse document.pdf --ocr-server-url http://localhost:8828/ocr --ocr-language zh
```

Or in code:

```typescript
import { LiteParse } from 'liteparse';

const parser = new LiteParse({
  ocrServerUrl: 'http://localhost:8828/ocr',
  ocrLanguage: 'en',
});

const result = await parser.parse('document.pdf');
```
